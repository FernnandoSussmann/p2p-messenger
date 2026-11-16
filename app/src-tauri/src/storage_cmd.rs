//! The storage status commands over the single encrypted database.
//!
//! The logic lives in plain functions so it can be tested without a Tauri
//! runtime; the `#[tauri::command]` wrappers only forward the managed state.
//! There is deliberately no reset, delete or recreate command: `storage_retry`
//! only tries to open again. Reasons are fixed strings, never a path or detail.

use std::sync::Arc;

use messenger_core::identity::{AccountId, IdentityService, IdentityStatus};
use messenger_core::identity::{KeychainStore, SettingsStore, TicketProvider};
use messenger_core::storage::{Storage, StorageError};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::chat_cmd::ChatState;
use crate::identity_cmd::AppIdentity;
use crate::net_cmd;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StorageErrorKind {
    KeyMissing,
    Missing,
    KeychainDenied,
    KeychainUnavailable,
    NotEncrypted,
    WrongKeyOrCorrupt,
    Corrupt,
    NewerSchema,
    MigrationFailed,
    OtherAccount,
    LegacyImportFailed,
    Io,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum StorageStatusDto {
    Ready,
    NotProvisioned,
    Error {
        kind: StorageErrorKind,
        reason: String,
    },
}

fn kind_of(err: StorageError) -> Option<StorageErrorKind> {
    Some(match err {
        StorageError::NotProvisioned => return None,
        StorageError::KeyMissing => StorageErrorKind::KeyMissing,
        StorageError::Missing => StorageErrorKind::Missing,
        StorageError::KeychainDenied => StorageErrorKind::KeychainDenied,
        StorageError::KeychainUnavailable => StorageErrorKind::KeychainUnavailable,
        StorageError::NotEncrypted => StorageErrorKind::NotEncrypted,
        StorageError::WrongKeyOrCorrupt => StorageErrorKind::WrongKeyOrCorrupt,
        StorageError::Corrupt => StorageErrorKind::Corrupt,
        StorageError::NewerSchema => StorageErrorKind::NewerSchema,
        StorageError::MigrationFailed { .. } => StorageErrorKind::MigrationFailed,
        StorageError::OtherAccount => StorageErrorKind::OtherAccount,
        StorageError::LegacyImportFailed => StorageErrorKind::LegacyImportFailed,
        StorageError::Io => StorageErrorKind::Io,
    })
}

fn reason(kind: StorageErrorKind) -> &'static str {
    match kind {
        StorageErrorKind::KeyMissing => {
            "The encryption key for your database is missing from your system keychain. Nothing was deleted or reset."
        }
        StorageErrorKind::Missing => {
            "Your encrypted database file is missing from the app data folder. A new empty one was not created, so nothing is overwritten if you restore it."
        }
        StorageErrorKind::KeychainDenied => {
            "Access to your system keychain was denied or the keychain is locked. Unlock it and retry."
        }
        StorageErrorKind::KeychainUnavailable => {
            "Your system keychain reported an error. Nothing was deleted or reset."
        }
        StorageErrorKind::NotEncrypted => {
            "Database encryption is not available in this build, so the database was not opened."
        }
        StorageErrorKind::WrongKeyOrCorrupt => {
            "The encrypted database could not be opened with the key in your system keychain, or the file is damaged. Nothing was deleted or reset."
        }
        StorageErrorKind::Corrupt => {
            "The encrypted database failed its integrity check. Nothing was deleted or reset."
        }
        StorageErrorKind::NewerSchema => "This database was written by a newer version of the app.",
        StorageErrorKind::MigrationFailed => {
            "Upgrading the database failed and was rolled back. Your data is unchanged."
        }
        StorageErrorKind::OtherAccount => {
            "The database is already open for a different account."
        }
        StorageErrorKind::LegacyImportFailed => {
            "Older unencrypted app data could not be read, so it was left in place and nothing was imported. Your encrypted data is unchanged."
        }
        StorageErrorKind::Io => {
            "The database file could not be accessed. Check the app data folder permissions and disk space."
        }
    }
}

impl From<StorageError> for StorageStatusDto {
    fn from(err: StorageError) -> Self {
        match kind_of(err) {
            Some(kind) => Self::Error {
                kind,
                reason: reason(kind).to_owned(),
            },
            None => Self::NotProvisioned,
        }
    }
}

/// Opens the store for `account` (idempotent) and reports the outcome.
pub fn open_for(storage: &Storage, chat: &ChatState, account: &AccountId) -> StorageStatusDto {
    match chat.open_store(storage, account) {
        Ok(()) => StorageStatusDto::Ready,
        Err(e) => e.into(),
    }
}

/// A present identity opens the store; an absent one, or one in an error
/// state (the identity screen owns that), is `NotProvisioned`.
pub fn status<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    identity: &IdentityService<K, S, T>,
    storage: &Storage,
    chat: &ChatState,
) -> StorageStatusDto {
    match identity.status() {
        IdentityStatus::Present { account_id, .. } => open_for(storage, chat, &account_id),
        IdentityStatus::Absent | IdentityStatus::Error(_) => StorageStatusDto::NotProvisioned,
    }
}

/// Clears a latched failure and tries again. Never deletes, resets or rekeys.
pub fn retry<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    identity: &IdentityService<K, S, T>,
    storage: &Storage,
    chat: &ChatState,
) -> StorageStatusDto {
    storage.retry();
    status(identity, storage, chat)
}

#[tauri::command]
pub fn storage_status(
    identity: State<'_, AppIdentity>,
    storage: State<'_, Arc<Storage>>,
    chat: State<'_, Arc<ChatState>>,
) -> StorageStatusDto {
    status(&identity, &storage, &chat)
}

#[tauri::command]
pub fn storage_retry(
    app: AppHandle,
    identity: State<'_, AppIdentity>,
    storage: State<'_, Arc<Storage>>,
    chat: State<'_, Arc<ChatState>>,
) -> StorageStatusDto {
    let outcome = retry(&identity, &storage, &chat);
    if outcome == StorageStatusDto::Ready {
        net_cmd::ensure_started(&app);
    }
    outcome
}

#[cfg(test)]
mod tests;
