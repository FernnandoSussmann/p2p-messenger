use rusqlite::{Connection, TransactionBehavior};

use super::StorageError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    /// 1, 2, 3, ... contiguous.
    pub version: u32,
    /// Logged, never user data.
    pub label: &'static str,
    /// Constant SQL owned by the slice.
    pub sql: &'static str,
}

/// Brings the DB to the last migration's version. Each migration runs in its
/// own IMMEDIATE transaction together with its `user_version` bump, so a
/// failure rolls back both and leaves the previous version intact. Re-running
/// is a no-op.
pub(crate) fn run(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, StorageError> {
    debug_assert!(is_contiguous(migrations), "migrations must be 1, 2, 3, ...");
    let latest = migrations.last().map_or(0, |m| m.version);
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| StorageError::Io)?;
    if current > latest {
        return Err(StorageError::NewerSchema);
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let failed = StorageError::MigrationFailed { version: m.version };
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failed)?;
        tx.execute_batch(m.sql).map_err(|_| failed)?;
        tx.pragma_update(None, "user_version", m.version)
            .map_err(|_| failed)?;
        tx.commit().map_err(|_| failed)?;
        tracing::info!(version = m.version, migration = m.label, "storage migrated");
    }
    Ok(latest)
}

pub(crate) fn is_contiguous(migrations: &[Migration]) -> bool {
    migrations
        .iter()
        .zip(1u32..)
        .all(|(m, expected)| m.version == expected)
}
