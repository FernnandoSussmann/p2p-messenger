use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use super::database::{Create, Database};
use super::schema::{MIGRATIONS, SCHEMA_VERSION};
use super::{StorageError, DB_FILE};
use crate::identity::{read_marker, AccountId, DbKey, KeychainError, KeychainStore};

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

/// A lazily opened, process-wide handle to the encrypted database.
pub struct Storage {
    data_dir: PathBuf,
    keys: Arc<dyn KeySource>,
    slot: Mutex<Slot>,
}

impl Storage {
    pub fn new(data_dir: impl Into<PathBuf>, keys: Arc<dyn KeySource>) -> Self {
        Self {
            data_dir: data_dir.into(),
            keys,
            slot: Mutex::new(Slot::Closed),
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
        match &*self.slot() {
            Slot::Closed => StorageStatus::NotOpened,
            Slot::Open { .. } => StorageStatus::Ready,
            Slot::Failed(e) => StorageStatus::Failed(*e),
        }
    }

    /// Clears a latched failure so the next `database` call opens again.
    pub fn retry(&self) {
        let mut slot = self.slot();
        if matches!(*slot, Slot::Failed(_)) {
            *slot = Slot::Closed;
        }
    }

    /// Test fixtures only: like `database`, but always `Create::Allowed`. The
    /// app never enables `test-support`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn database_creating(&self, account: &AccountId) -> Result<Arc<Database>, StorageError> {
        self.open_with(account, true)
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn open_with(
        &self,
        account: &AccountId,
        force_create: bool,
    ) -> Result<Arc<Database>, StorageError> {
        let mut slot = self.slot();
        match &*slot {
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
        match self.try_open(account, force_create) {
            Ok(db) => {
                tracing::info!(schema = SCHEMA_VERSION, "storage opened");
                *slot = Slot::Open {
                    account: *account,
                    db: Arc::clone(&db),
                };
                Ok(db)
            }
            Err(e) => {
                tracing::warn!(error = %e, "storage unavailable");
                *slot = Slot::Failed(e);
                Err(e)
            }
        }
    }

    fn try_open(
        &self,
        account: &AccountId,
        force_create: bool,
    ) -> Result<Arc<Database>, StorageError> {
        let key = self.keys.db_key(account)?;
        let create = if force_create || self.may_create() {
            Create::Allowed
        } else {
            Create::Forbidden
        };
        Database::open(&self.data_dir.join(DB_FILE), &key, MIGRATIONS, create).map(Arc::new)
    }

    /// Creation is allowed only while provisioning (no marker yet) or while
    /// legacy plaintext data awaits import. Otherwise an absent file is
    /// `Missing`: an empty replacement would look exactly like silent data loss.
    fn may_create(&self) -> bool {
        matches!(read_marker(&self.data_dir), Ok(None))
            || LEGACY_FILES.iter().any(|f| self.data_dir.join(f).exists())
    }
}
