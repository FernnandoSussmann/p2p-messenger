use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags};
use zeroize::Zeroizing;

use super::migrate::{self, Migration};
use super::StorageError;
use crate::identity::DbKey;

/// Whether `Database::open` may create an absent file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Create {
    Allowed,
    Forbidden,
}

/// The open, keyed, migrated connection. Slices run their constant SQL through
/// [`Database::conn`].
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// [`Database::open_reporting`] without the `created` report.
    #[cfg(test)]
    pub(crate) fn open(
        path: &Path,
        key: &DbKey,
        migrations: &[Migration],
        create: Create,
    ) -> Result<Self, StorageError> {
        Self::open_reporting(path, key, migrations, create, &mut false)
    }

    /// Opens the keyed DB at `path` and migrates it with `migrations`. Creates
    /// the file only when `create` is `Create::Allowed`; an absent file
    /// otherwise is `Missing`. Never truncates, deletes or recreates an
    /// existing file. `created` is set as soon as this call created the file,
    /// even if the open fails afterwards.
    pub(crate) fn open_reporting(
        path: &Path,
        key: &DbKey,
        migrations: &[Migration],
        create: Create,
        created: &mut bool,
    ) -> Result<Self, StorageError> {
        let state = ensure_private_regular_file(path, create)?;
        *created = state.created;
        let had_data = state.had_data;
        // No SQLITE_OPEN_CREATE (the file exists already) and no SQLITE_OPEN_URI.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = Connection::open_with_flags(path, flags).map_err(|_| StorageError::Io)?;
        apply_key(&conn, key)?;
        let version: Option<String> = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .ok();
        if version.is_none_or(|v| v.is_empty()) {
            return Err(StorageError::NotEncrypted);
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| StorageError::WrongKeyOrCorrupt)?;
        if had_data {
            integrity_check(&conn)?;
        }
        conn.execute_batch(
            "PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON; PRAGMA foreign_keys = ON;",
        )
        .map_err(|_| StorageError::Io)?;
        verify_session_pragmas(&conn)?;
        migrate::run(&mut conn, migrations)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::Io)
    }

    /// Rebuilds the file so no free page can hold deleted ciphertext. Must run
    /// outside any transaction and without the guard from [`Database::conn`]
    /// held. It holds the connection for the whole rebuild and, with
    /// `temp_store=MEMORY`, keeps the rebuild image in RAM, so async callers
    /// must use `spawn_blocking`.
    pub(crate) fn reclaim(&self) -> Result<(), StorageError> {
        let conn = self.conn()?;
        conn.execute_batch("VACUUM;")
            .map_err(|_| StorageError::Io)?;
        let free: i64 = conn
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .map_err(|_| StorageError::Io)?;
        if free == 0 {
            Ok(())
        } else {
            Err(StorageError::Io)
        }
    }
}

/// The key is built from the fixed hex alphabet only, as a raw 256-bit blob
/// literal (no KDF), and dropped right after the statement runs.
fn apply_key(conn: &Connection, key: &DbKey) -> Result<(), StorageError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut sql = Zeroizing::new(String::with_capacity(16 + 64 + 3));
    sql.push_str("PRAGMA key = \"x'");
    for b in key.as_bytes() {
        sql.push(HEX[usize::from(b >> 4)] as char);
        sql.push(HEX[usize::from(b & 0x0f)] as char);
    }
    sql.push_str("'\";");
    conn.execute_batch(&sql).map_err(|_| StorageError::Io)?;
    drop(sql);
    Ok(())
}

/// `cipher_integrity_check` returns one row per problem; `quick_check` returns
/// `ok` as its first row when healthy.
fn integrity_check(conn: &Connection) -> Result<(), StorageError> {
    let problem = conn
        .prepare("PRAGMA cipher_integrity_check")
        .and_then(|mut s| s.query([])?.next().map(|row| row.is_some()))
        .map_err(|_| StorageError::Corrupt)?;
    if problem {
        return Err(StorageError::Corrupt);
    }
    let first: Option<String> = conn
        .prepare("PRAGMA quick_check")
        .and_then(|mut s| s.query([])?.next()?.map(|r| r.get(0)).transpose())
        .map_err(|_| StorageError::Corrupt)?;
    if first.as_deref() != Some("ok") {
        return Err(StorageError::Corrupt);
    }
    Ok(())
}

fn verify_session_pragmas(conn: &Connection) -> Result<(), StorageError> {
    let read = |name: &str| -> Result<i64, StorageError> {
        conn.pragma_query_value(None, name, |r| r.get(0))
            .map_err(|_| StorageError::Io)
    };
    // temp_store: 2 = MEMORY.
    if read("secure_delete")? != 1 || read("temp_store")? != 2 {
        return Err(StorageError::Io);
    }
    Ok(())
}

struct FileState {
    /// The file held data before this call.
    had_data: bool,
    /// This call created the file.
    created: bool,
}

/// Refuses symlinks and non-regular files; creates the file `0600` only when
/// `create` allows it and tightens loose modes on an existing file (Unix).
fn ensure_private_regular_file(path: &Path, create: Create) -> Result<FileState, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                return Err(StorageError::Io);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o077 != 0 {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                        .map_err(|_| StorageError::Io)?;
                }
            }
            Ok(FileState {
                had_data: meta.len() > 0,
                created: false,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if create == Create::Forbidden {
                return Err(StorageError::Missing);
            }
            create_new_private(path)?;
            Ok(FileState {
                had_data: false,
                created: true,
            })
        }
        Err(_) => Err(StorageError::Io),
    }
}

fn create_new_private(path: &Path) -> Result<(), StorageError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map(drop).map_err(|_| StorageError::Io)
}
