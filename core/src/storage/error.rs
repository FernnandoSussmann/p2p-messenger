use std::fmt;

use crate::identity::KeychainError;

/// Why the encrypted store could not be opened. Deliberately payload-free: a
/// payload could carry a backend string, a path or key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// No identity exists yet, so there is nothing to open.
    NotProvisioned,
    /// The DB key is missing from the keychain.
    KeyMissing,
    /// An identity exists but `messenger.db` does not, and creating one is not
    /// allowed. Never silently recreated.
    Missing,
    /// The keychain is locked or the user refused access.
    KeychainDenied,
    /// Any other keychain failure (backend detail deliberately dropped).
    KeychainUnavailable,
    /// The SQLite library has no SQLCipher codec.
    NotEncrypted,
    /// The key does not decrypt the file, or its first page is damaged.
    WrongKeyOrCorrupt,
    /// The key decrypts, but the integrity check failed.
    Corrupt,
    /// The file was written by a newer app version.
    NewerSchema,
    /// Migration `version` failed; it was rolled back and the prior version kept.
    MigrationFailed { version: u32 },
    /// The handle is already open for a different account (v1 has one account).
    OtherAccount,
    /// File-system / SQLite I/O failure (permissions, disk full, symlink, lock).
    Io,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotProvisioned => "no identity exists yet",
            Self::KeyMissing => "the database key is missing from the keychain",
            Self::Missing => "the encrypted database file is missing",
            Self::KeychainDenied => "keychain access was denied",
            Self::KeychainUnavailable => "the keychain is unavailable",
            Self::NotEncrypted => "database encryption is not available",
            Self::WrongKeyOrCorrupt => "the database key is wrong or the file is corrupt",
            Self::Corrupt => "the database failed its integrity check",
            Self::NewerSchema => "the database was written by a newer version",
            Self::MigrationFailed { .. } => "a database migration failed and was rolled back",
            Self::OtherAccount => "the database is open for a different account",
            Self::Io => "the database file could not be accessed",
        })
    }
}

impl std::error::Error for StorageError {}

impl From<KeychainError> for StorageError {
    fn from(e: KeychainError) -> Self {
        match e {
            KeychainError::NotFound => Self::KeyMissing,
            KeychainError::AccessDenied => Self::KeychainDenied,
            KeychainError::Backend(_) => Self::KeychainUnavailable,
        }
    }
}
