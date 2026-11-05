//! The DD-05 seam for per-account settings.
//!
//! [`MemorySettingsStore`] is a narrow placeholder that the DD-05 slice will
//! replace with a SQLCipher-backed store opened with the `DbKey`. It holds the
//! display name and provisioning timestamp only, and never any secret.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use super::{AccountId, DisplayName};

#[derive(Debug)]
pub enum SettingsError {
    /// The backing store could not be read or written.
    Unavailable,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Unavailable => write!(f, "settings store unavailable"),
        }
    }
}

impl std::error::Error for SettingsError {}

pub trait SettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError>;
    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError>;
    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError>;
    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError>;
}

#[derive(Default)]
struct AccountSettings {
    display_name: Option<DisplayName>,
    provisioned_at: Option<i64>,
}

#[derive(Default)]
pub struct MemorySettingsStore {
    accounts: Mutex<HashMap<uuid::Uuid, AccountSettings>>,
}

impl MemorySettingsStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_account<T>(
        &self,
        id: &AccountId,
        f: impl FnOnce(&mut AccountSettings) -> T,
    ) -> Result<T, SettingsError> {
        let mut accounts = self
            .accounts
            .lock()
            .map_err(|_| SettingsError::Unavailable)?;
        Ok(f(accounts.entry(id.as_uuid()).or_default()))
    }
}

impl SettingsStore for MemorySettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        self.with_account(id, |s| s.display_name = Some(name.clone()))
    }

    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        self.with_account(id, |s| s.display_name.clone())
    }

    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        self.with_account(id, |s| s.provisioned_at = Some(unix_ms))
    }

    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        self.with_account(id, |s| s.provisioned_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::validate_display_name;

    fn name(raw: &str) -> DisplayName {
        validate_display_name(raw).unwrap()
    }

    #[test]
    fn set_get_display_name_round_trips() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Ada"
        );
    }

    #[test]
    fn overwrite_display_name() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_display_name(&id, &name("Grace")).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Grace"
        );
    }

    #[test]
    fn provisioned_at_round_trips() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_provisioned_at(&id, 1_700_000_000_000).unwrap();
        assert_eq!(
            store.get_provisioned_at(&id).unwrap(),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn missing_returns_none() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        assert!(store.get_display_name(&id).unwrap().is_none());
        assert!(store.get_provisioned_at(&id).unwrap().is_none());
    }

    #[test]
    fn accounts_are_isolated() {
        let store = MemorySettingsStore::new();
        let (a, b) = (AccountId::new(), AccountId::new());
        store.set_display_name(&a, &name("Ada")).unwrap();
        assert!(store.get_display_name(&b).unwrap().is_none());
    }
}
