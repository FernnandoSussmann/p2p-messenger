//! One-shot import of the pre-DD-05 plaintext files (`settings.json`,
//! `contacts.json`, `known_peers.json`) into the encrypted database.
//!
//! `Storage` runs [`import_all`] once, right after the first successful
//! `Database::open` and before the handle is returned, so nothing can observe a
//! half-imported state. The import works only on the `&Database` it is given:
//! it never calls `Storage`, a store trait method or anything else that
//! re-enters the slot mutex or the connection mutex while it holds the guard.
//!
//! Per file: parse (readers never touch the database), then ONE IMMEDIATE
//! transaction inserts the rows (insert-if-absent) together with the
//! `legacy_imports` row, then, after the commit and with the guard released,
//! the plaintext file is overwritten with zeros, synced and unlinked. The
//! `legacy_imports` row guarantees a file that survived a failed unlink is
//! never imported again, so a contact the user removed is never resurrected.
//!
//! A corrupt or non-regular `settings.json` / `contacts.json` is
//! [`StorageError::LegacyImportFailed`]: nothing is imported and the file is
//! left untouched. A corrupt `known_peers.json` is skipped and **stays on
//! disk as plaintext** (those peers are prompted again); the user can delete
//! it. Zero-overwriting does not guarantee erasure on SSDs, copy-on-write file
//! systems (APFS, btrfs), or in backups and snapshots.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::{Database, StorageError};
use crate::contacts::{self, Contact, MAX_CONTACTS_PER_ACCOUNT};
use crate::identity::{self, AccountId, LegacySettings};

/// Install-level bookkeeping: the one documented exception (next to
/// `user_version`) to "every row carries `account_id`".
pub(crate) const MIGRATION_LEGACY_IMPORTS: &str = "
CREATE TABLE legacy_imports (
  file           TEXT    PRIMARY KEY CHECK (file IN ('settings.json', 'contacts.json', 'known_peers.json')),
  imported_at_ms INTEGER NOT NULL
);
";

const SQL_LEGACY_SEEN: &str = "SELECT 1 FROM legacy_imports WHERE file = ?1";
const SQL_LEGACY_RECORD: &str = "INSERT INTO legacy_imports (file, imported_at_ms) VALUES (?1, ?2)";

/// Why a legacy plaintext file could not be read. No paths, no contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyError {
    /// Present but not a regular file (symlink, directory, device, ...). Never followed.
    NotRegularFile,
    /// Unparseable, wrong version, over the size cap, or a field fails validation.
    Corrupt,
    /// Present but unreadable (permissions, I/O).
    Io,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ImportReport {
    /// `account_settings` rows inserted.
    pub settings_accounts: usize,
    /// `contacts` rows inserted from `contacts.json`.
    pub contacts: usize,
    /// `contacts` rows inserted from `known_peers.json`.
    pub known_peers: usize,
    /// `known_peers.json` was present but unreadable and is left on disk.
    pub known_peers_skipped: bool,
    /// Committed imports whose plaintext file could not be removed yet.
    pub removals_pending: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Settings,
    Contacts,
    KnownPeers,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Settings, Kind::Contacts, Kind::KnownPeers];

    fn file(self) -> &'static str {
        match self {
            Kind::Settings => "settings.json",
            Kind::Contacts => "contacts.json",
            Kind::KnownPeers => "known_peers.json",
        }
    }

    /// A broken `known_peers.json` never blocks.
    fn fatal(self) -> bool {
        self != Kind::KnownPeers
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Settings => "settings",
            Kind::Contacts => "contacts",
            Kind::KnownPeers => "known_peers",
        }
    }
}

enum Parsed {
    Settings(Vec<(AccountId, LegacySettings)>),
    Contacts(Vec<(AccountId, Contact)>),
}

fn parse(kind: Kind, data_dir: &Path) -> Result<Option<Parsed>, LegacyError> {
    Ok(match kind {
        Kind::Settings => identity::read_legacy_settings(data_dir)?.map(Parsed::Settings),
        Kind::Contacts => contacts::read_legacy_contacts(data_dir)?.map(Parsed::Contacts),
        Kind::KnownPeers => contacts::read_legacy_known_peers(data_dir)?.map(Parsed::Contacts),
    })
}

pub(crate) fn import_all(db: &Database, data_dir: &Path) -> Result<ImportReport, StorageError> {
    let mut report = ImportReport::default();
    for kind in Kind::ALL {
        import_one(db, data_dir, kind, &mut report)?;
    }
    Ok(report)
}

fn import_one(
    db: &Database,
    data_dir: &Path,
    kind: Kind,
    report: &mut ImportReport,
) -> Result<(), StorageError> {
    let failed = StorageError::LegacyImportFailed;
    let seen = {
        let conn = db.conn()?;
        already_imported(&conn, kind).map_err(|_| failed)?
    };
    if !seen {
        let parsed = match parse(kind, data_dir) {
            Ok(None) => return Ok(()),
            Ok(Some(parsed)) => parsed,
            Err(e) if kind.fatal() => {
                tracing::warn!(error = ?e, kind = kind.label(), "legacy data unreadable");
                return Err(failed);
            }
            Err(e) => {
                tracing::warn!(error = ?e, "legacy known peers skipped");
                report.known_peers_skipped = true;
                return Ok(());
            }
        };
        let inserted = {
            let mut conn = db.conn()?;
            commit_import(&mut conn, kind, &parsed)
        };
        match inserted {
            Ok(n) => {
                match kind {
                    Kind::Settings => report.settings_accounts += n,
                    Kind::Contacts => report.contacts += n,
                    Kind::KnownPeers => report.known_peers += n,
                }
                if n > 0 {
                    tracing::info!(count = n, kind = kind.label(), "legacy data imported");
                }
            }
            Err(_) if kind.fatal() => return Err(failed),
            Err(_) => {
                tracing::warn!("legacy known peers skipped");
                report.known_peers_skipped = true;
                return Ok(());
            }
        }
    }
    remove_after_commit(data_dir, kind, report);
    Ok(())
}

fn already_imported(conn: &Connection, kind: Kind) -> rusqlite::Result<bool> {
    conn.query_row(SQL_LEGACY_SEEN, params![kind.file()], |_| Ok(()))
        .optional()
        .map(|r| r.is_some())
}

/// Inserts the rows and the `legacy_imports` marker in one IMMEDIATE
/// transaction; any failure rolls everything back.
fn commit_import(conn: &mut Connection, kind: Kind, parsed: &Parsed) -> Result<usize, ()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(drop)?;
    let inserted = match parsed {
        Parsed::Settings(rows) => insert_settings(&tx, rows)?,
        Parsed::Contacts(rows) => insert_contacts(&tx, rows)?,
    };
    tx.execute(SQL_LEGACY_RECORD, params![kind.file(), now_ms()])
        .map_err(drop)?;
    tx.commit().map_err(drop)?;
    Ok(inserted)
}

fn insert_settings(tx: &Transaction, rows: &[(AccountId, LegacySettings)]) -> Result<usize, ()> {
    let mut inserted = 0;
    for (account, settings) in rows {
        if identity::insert_settings_if_absent_tx(tx, account, settings).map_err(drop)? {
            inserted += 1;
        }
    }
    Ok(inserted)
}

fn insert_contacts(tx: &Transaction, rows: &[(AccountId, Contact)]) -> Result<usize, ()> {
    let mut inserted = 0;
    let mut touched = Vec::new();
    for (account, contact) in rows {
        if contacts::insert_if_absent_tx(tx, account, contact).map_err(drop)? {
            inserted += 1;
        }
        if !touched.contains(account) {
            touched.push(*account);
        }
    }
    for account in &touched {
        if contacts::count_tx(tx, account).map_err(drop)? > MAX_CONTACTS_PER_ACCOUNT {
            return Err(());
        }
    }
    Ok(inserted)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

/// Runs with no database guard held. A failure is counted and retried at the
/// next start; the `legacy_imports` row keeps the file from being re-imported.
fn remove_after_commit(data_dir: &Path, kind: Kind, report: &mut ImportReport) {
    let path = data_dir.join(kind.file());
    let mut tmp = path.clone().into_os_string();
    tmp.push(".tmp");
    let mut pending = remove_plaintext(&path).is_err();
    pending |= remove_plaintext(Path::new(&tmp)).is_err();
    if pending {
        tracing::warn!(kind = kind.label(), "legacy plaintext file not removed");
        report.removals_pending += 1;
    }
}

/// Overwrites with zeros, syncs and unlinks a regular file. A symlink or other
/// non-regular path is never followed or unlinked. A file with more than one
/// hard link is only unlinked, never zero-filled, so the other link keeps its
/// content.
fn remove_plaintext(path: &Path) -> Result<(), ()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(()),
    };
    if !meta.file_type().is_file() {
        return Err(());
    }
    if sole_link(&meta) {
        zero_fill(path, meta.len()).map_err(drop)?;
    } else {
        tracing::warn!("legacy file has other hard links; unlinked without overwrite");
    }
    fs::remove_file(path).map_err(drop)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sole_link(meta: &fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(meta) == 1
}

#[cfg(not(unix))]
fn sole_link(_: &fs::Metadata) -> bool {
    true
}

fn zero_fill(path: &Path, len: u64) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    let zeros = [0u8; 8192];
    let mut left = len;
    while left > 0 {
        let n = left.min(zeros.len() as u64) as usize;
        f.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    f.sync_all()
}

#[cfg(test)]
mod tests;
