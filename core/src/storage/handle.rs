use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::database::{Create, Database};
use super::legacy;
use super::schema::{MIGRATIONS, SCHEMA_VERSION};
use super::{StorageError, DB_FILE};
use crate::contacts::SQL_CONTACTS_COUNT_ALL;
use crate::identity::{
    read_marker, AccountId, DbKey, KeychainError, KeychainStore, SQL_SETTINGS_COUNT_OTHERS,
};

/// Rows of any account: user data that provisioning must not discard. The
/// predicate is always true (the column is NOT NULL); it names the namespace
/// column to keep the scoping guard honest.
const SQL_MESSAGES_COUNT_ALL: &str = "SELECT count(*) FROM messages WHERE account_id IS NOT NULL";

/// Loads the DB key for an account. Blanket-implemented for every keychain, so
/// the app passes its keychain `Arc` without ever naming the key type. The
/// method is not called `load_db_key`: with the blanket impl, two methods of
/// that name in scope would make `keychain.load_db_key(..)` ambiguous.
pub trait KeySource: Send + Sync {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError>;
}

impl<K: KeychainStore + Send + Sync + ?Sized> KeySource for K {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError> {
        KeychainStore::load_db_key(self, account)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageStatus {
    NotOpened,
    Ready,
    Failed(StorageError),
}

/// Plaintext files of the pre-DD-05 interim stores; while any exists, the DB
/// may be created because legacy data is being imported.
const LEGACY_FILES: [&str; 3] = ["settings.json", "contacts.json", "known_peers.json"];

enum Slot {
    Closed,
    Open {
        account: AccountId,
        db: Arc<Database>,
    },
    Failed(StorageError),
}

struct State {
    slot: Slot,
    /// The account whose open created `messenger.db`, even if that open
    /// failed later. Only a file created by the failed provisioning attempt
    /// may be discarded.
    last_created: Option<AccountId>,
    /// A discard that found another live handle; retried by the next open.
    pending_discard: Option<(AccountId, Weak<Database>)>,
}

/// A lazily opened, process-wide handle to the encrypted database.
pub struct Storage {
    data_dir: PathBuf,
    keys: Arc<dyn KeySource>,
    state: Mutex<State>,
}

impl Storage {
    pub fn new(data_dir: impl Into<PathBuf>, keys: Arc<dyn KeySource>) -> Self {
        Self {
            data_dir: data_dir.into(),
            keys,
            state: Mutex::new(State {
                slot: Slot::Closed,
                last_created: None,
                pending_discard: None,
            }),
        }
    }

    /// Opens on first call (keychain, key, `Database::open` with the global
    /// migrations). A failure is latched: later calls return it without
    /// touching the keychain again, until [`Storage::retry`]. A different
    /// account while open is `OtherAccount`.
    pub fn database(&self, account: &AccountId) -> Result<Arc<Database>, StorageError> {
        self.open_with(account, false)
    }

    pub fn status(&self) -> StorageStatus {
        match &self.state().slot {
            Slot::Closed => StorageStatus::NotOpened,
            Slot::Open { .. } => StorageStatus::Ready,
            Slot::Failed(e) => StorageStatus::Failed(*e),
        }
    }

    /// Clears a latched failure so the next `database` call opens again.
    pub fn retry(&self) {
        let mut state = self.state();
        if matches!(state.slot, Slot::Failed(_)) {
            state.slot = Slot::Closed;
        }
    }

    /// Test fixtures only: like `database`, but always `Create::Allowed`. The
    /// app never enables `test-support`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn database_creating(&self, account: &AccountId) -> Result<Arc<Database>, StorageError> {
        self.open_with(account, true)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn open_with(
        &self,
        account: &AccountId,
        force_create: bool,
    ) -> Result<Arc<Database>, StorageError> {
        let mut state = self.state();
        self.retry_pending_discard(&mut state)?;
        match &state.slot {
            Slot::Open { account: a, db } => {
                return if a == account {
                    Ok(Arc::clone(db))
                } else {
                    Err(StorageError::OtherAccount)
                };
            }
            Slot::Failed(e) => return Err(*e),
            Slot::Closed => {}
        }
        let mut created = false;
        let opened = self.try_open(account, force_create, &mut created);
        if created {
            state.last_created = Some(*account);
        }
        match opened {
            Ok(db) => {
                tracing::info!(schema = SCHEMA_VERSION, "storage opened");
                state.slot = Slot::Open {
                    account: *account,
                    db: Arc::clone(&db),
                };
                Ok(db)
            }
            Err(e) => {
                tracing::warn!(error = %e, "storage unavailable");
                state.slot = Slot::Failed(e);
                Err(e)
            }
        }
    }

    fn try_open(
        &self,
        account: &AccountId,
        force_create: bool,
        created: &mut bool,
    ) -> Result<Arc<Database>, StorageError> {
        let key = self.keys.db_key(account)?;
        let create = if force_create || self.may_create() {
            Create::Allowed
        } else {
            Create::Forbidden
        };
        let db = Database::open_reporting(
            &self.data_dir.join(DB_FILE),
            &key,
            MIGRATIONS,
            create,
            created,
        )?;
        legacy::import_all(&db, &self.data_dir)?;
        Ok(Arc::new(db))
    }

    /// Undoes the storage side of a failed provisioning of `account`.
    ///
    /// Always resets the slot to `Closed`, so the next attempt (a new
    /// `AccountId`) never meets a stale latched failure or `OtherAccount`.
    /// Deletes the file only if this attempt created it, no marker exists, it
    /// holds no user data and nobody else holds the handle. Anything else is
    /// kept. If only the sole-handle check fails, the deletion is retried by
    /// the next `database` call once the other handle is gone; until then that
    /// call returns an unlatched `Io`.
    pub(crate) fn discard_provisioning(&self, account: &AccountId) -> Result<(), StorageError> {
        let mut state = self.state();
        let slot = std::mem::replace(&mut state.slot, Slot::Closed);
        if state.last_created != Some(*account) || !matches!(read_marker(&self.data_dir), Ok(None))
        {
            return Ok(());
        }
        let path = self.data_dir.join(DB_FILE);
        match slot {
            Slot::Open { db, .. } => {
                if holds_user_data(&db, account)? {
                    return Ok(());
                }
                match Arc::try_unwrap(db) {
                    Ok(db) => {
                        drop(db);
                        self.delete_files(&mut state)
                    }
                    Err(shared) => {
                        state.pending_discard = Some((*account, Arc::downgrade(&shared)));
                        Ok(())
                    }
                }
            }
            Slot::Failed(_) => match fs::metadata(&path) {
                Ok(m) if m.is_file() && m.len() == 0 => self.delete_files(&mut state),
                _ => Ok(()),
            },
            Slot::Closed => Ok(()),
        }
    }

    fn delete_files(&self, state: &mut State) -> Result<(), StorageError> {
        let base = self.data_dir.join(DB_FILE);
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut name = base.clone().into_os_string();
            name.push(suffix);
            match fs::remove_file(&name) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(StorageError::Io),
            }
        }
        state.last_created = None;
        state.pending_discard = None;
        Ok(())
    }

    /// While the marker is still absent, finishes a deferred discard once the
    /// extra handle is gone; while it is alive, fails with an unlatched `Io`.
    fn retry_pending_discard(&self, state: &mut State) -> Result<(), StorageError> {
        let Some((_, weak)) = &state.pending_discard else {
            return Ok(());
        };
        let alive = weak.strong_count() > 0;
        if !matches!(read_marker(&self.data_dir), Ok(None)) {
            state.pending_discard = None;
            Ok(())
        } else if alive {
            Err(StorageError::Io)
        } else {
            self.delete_files(state)
        }
    }

    /// Creation is allowed only while provisioning (no marker yet) or while
    /// legacy plaintext data awaits import. Otherwise an absent file is
    /// `Missing`: an empty replacement would look exactly like silent data loss.
    fn may_create(&self) -> bool {
        matches!(read_marker(&self.data_dir), Ok(None))
            || LEGACY_FILES.iter().any(|f| self.data_dir.join(f).exists())
    }
}

/// Any message, contact or other account's settings: data a failed
/// provisioning must never delete.
fn holds_user_data(db: &Database, account: &AccountId) -> Result<bool, StorageError> {
    let conn = db.conn()?;
    let count = |sql: &str, others: bool| -> Result<i64, StorageError> {
        let result = if others {
            conn.query_row(sql, [account.as_uuid().as_bytes().as_slice()], |r| r.get(0))
        } else {
            conn.query_row(sql, [], |r| r.get(0))
        };
        result.map_err(|_| StorageError::Io)
    };
    Ok(count(SQL_MESSAGES_COUNT_ALL, false)? != 0
        || count(SQL_CONTACTS_COUNT_ALL, false)? != 0
        || count(SQL_SETTINGS_COUNT_OTHERS, true)? != 0)
}
