//! The OS-keychain boundary: the only place the two identity secrets are persisted.
//!
//! Entries live under one service ([`APP_KEYCHAIN_SERVICE`], or a profile's own
//! service) and are keyed per
//! account as `<account_id>/identity-key` and `<account_id>/db-key`.

use std::fmt;
use std::sync::Arc;

use keyring_core::Entry;
use zeroize::Zeroizing;

use super::{AccountId, DbKey, Profile, SecretKeyBytes};

pub const APP_KEYCHAIN_SERVICE: &str = "com.p2p-messenger.identity";

const IDENTITY_KEY_SUFFIX: &str = "identity-key";
const DB_KEY_SUFFIX: &str = "db-key";

/// Why a keychain operation failed. Callers must treat `AccessDenied` and
/// `Backend` as error states, never as "no identity yet".
#[derive(Debug)]
pub enum KeychainError {
    /// No entry exists for this account.
    NotFound,
    /// The keychain is locked or the user refused access.
    AccessDenied,
    /// Any other backend failure, including malformed stored data.
    Backend(String),
}

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeychainError::NotFound => write!(f, "keychain entry not found"),
            KeychainError::AccessDenied => write!(f, "keychain access denied"),
            KeychainError::Backend(msg) => write!(f, "keychain backend error: {msg}"),
        }
    }
}

impl std::error::Error for KeychainError {}

impl From<keyring_core::Error> for KeychainError {
    fn from(err: keyring_core::Error) -> Self {
        match err {
            keyring_core::Error::NoEntry => KeychainError::NotFound,
            keyring_core::Error::NoStorageAccess(_) => KeychainError::AccessDenied,
            other => KeychainError::Backend(other.to_string()),
        }
    }
}

pub trait KeychainStore {
    fn store_identity_key(&self, id: &AccountId, key: &SecretKeyBytes)
        -> Result<(), KeychainError>;
    fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError>;
    fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError>;
    fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError>;
    fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError>;
    fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError>;
}

/// Lets the identity service and other slices share one keychain handle.
impl<T: KeychainStore + ?Sized> KeychainStore for Arc<T> {
    fn store_identity_key(
        &self,
        id: &AccountId,
        key: &SecretKeyBytes,
    ) -> Result<(), KeychainError> {
        (**self).store_identity_key(id, key)
    }

    fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError> {
        (**self).store_db_key(id, key)
    }

    fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError> {
        (**self).load_identity_key(id)
    }

    fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError> {
        (**self).load_db_key(id)
    }

    fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        (**self).delete_identity_key(id)
    }

    fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        (**self).delete_db_key(id)
    }
}

/// [`KeychainStore`] backed by the default `keyring` credential store.
#[derive(Debug)]
pub struct OsKeychain {
    service: String,
}

impl OsKeychain {
    /// Initialises the platform credential store and returns a handle to it.
    pub fn native() -> Result<Self, KeychainError> {
        Self::native_for(None)
    }

    /// Like [`OsKeychain::native`], but under the profile's own service name
    /// when a profile is given. `None` is exactly `native()`.
    pub fn native_for(profile: Option<&Profile>) -> Result<Self, KeychainError> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(Self {
                service: profile.map_or_else(
                    || APP_KEYCHAIN_SERVICE.to_owned(),
                    Profile::keychain_service,
                ),
            }),
            Err(err) => Err(KeychainError::Backend(err.to_string())),
        }
    }

    fn entry(&self, id: &AccountId, suffix: &str) -> Result<Entry, KeychainError> {
        Ok(Entry::new(&self.service, &account_name(id, suffix))?)
    }

    fn load_key(&self, id: &AccountId, suffix: &str) -> Result<Zeroizing<[u8; 32]>, KeychainError> {
        let raw = Zeroizing::new(self.entry(id, suffix)?.get_secret()?);
        let bytes = <[u8; 32]>::try_from(raw.as_slice())
            .map_err(|_| KeychainError::Backend("stored key has wrong length".into()))?;
        Ok(Zeroizing::new(bytes))
    }
}

fn account_name(id: &AccountId, suffix: &str) -> String {
    format!("{}/{suffix}", id.as_uuid())
}

impl KeychainStore for OsKeychain {
    fn store_identity_key(
        &self,
        id: &AccountId,
        key: &SecretKeyBytes,
    ) -> Result<(), KeychainError> {
        Ok(self
            .entry(id, IDENTITY_KEY_SUFFIX)?
            .set_secret(key.as_bytes())?)
    }

    fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError> {
        Ok(self.entry(id, DB_KEY_SUFFIX)?.set_secret(key.as_bytes())?)
    }

    fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError> {
        Ok(SecretKeyBytes::from_bytes(
            *self.load_key(id, IDENTITY_KEY_SUFFIX)?,
        ))
    }

    fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError> {
        Ok(DbKey::from_bytes(*self.load_key(id, DB_KEY_SUFFIX)?))
    }

    fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        Ok(self.entry(id, IDENTITY_KEY_SUFFIX)?.delete_credential()?)
    }

    fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        Ok(self.entry(id, DB_KEY_SUFFIX)?.delete_credential()?)
    }
}

/// In-memory fakes shared with integration tests under `core/tests/`.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// A [`KeychainStore`] held in memory that can be told to fail.
    #[derive(Default)]
    pub struct FakeKeychain {
        entries: Mutex<HashMap<String, [u8; 32]>>,
        fail_with: Mutex<Option<fn() -> KeychainError>>,
    }

    impl FakeKeychain {
        pub fn new() -> Self {
            Self::default()
        }

        /// Whether no secret is stored.
        pub fn is_empty(&self) -> bool {
            self.entries.lock().unwrap().is_empty()
        }

        /// Makes every later operation return the error built by `make_error`.
        pub fn fail_with(&self, make_error: fn() -> KeychainError) {
            *self.fail_with.lock().unwrap() = Some(make_error);
        }

        /// Overwrites the stored DB key with a random one (simulates a wrong key).
        pub fn replace_db_key_with_random(&self, id: &AccountId) {
            self.entries
                .lock()
                .unwrap()
                .insert(account_name(id, DB_KEY_SUFFIX), rand::random::<[u8; 32]>());
        }

        fn check(&self) -> Result<(), KeychainError> {
            match *self.fail_with.lock().unwrap() {
                Some(make_error) => Err(make_error()),
                None => Ok(()),
            }
        }

        fn put(&self, id: &AccountId, suffix: &str, bytes: &[u8; 32]) -> Result<(), KeychainError> {
            self.check()?;
            self.entries
                .lock()
                .unwrap()
                .insert(account_name(id, suffix), *bytes);
            Ok(())
        }

        fn get(&self, id: &AccountId, suffix: &str) -> Result<[u8; 32], KeychainError> {
            self.check()?;
            let entries = self.entries.lock().unwrap();
            entries
                .get(&account_name(id, suffix))
                .copied()
                .ok_or(KeychainError::NotFound)
        }

        fn remove(&self, id: &AccountId, suffix: &str) -> Result<(), KeychainError> {
            self.check()?;
            let removed = self
                .entries
                .lock()
                .unwrap()
                .remove(&account_name(id, suffix));
            removed.map(|_| ()).ok_or(KeychainError::NotFound)
        }
    }

    impl KeychainStore for FakeKeychain {
        fn store_identity_key(
            &self,
            id: &AccountId,
            key: &SecretKeyBytes,
        ) -> Result<(), KeychainError> {
            self.put(id, IDENTITY_KEY_SUFFIX, key.as_bytes())
        }

        fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError> {
            self.put(id, DB_KEY_SUFFIX, key.as_bytes())
        }

        fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError> {
            self.get(id, IDENTITY_KEY_SUFFIX)
                .map(SecretKeyBytes::from_bytes)
        }

        fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError> {
            self.get(id, DB_KEY_SUFFIX).map(DbKey::from_bytes)
        }

        fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError> {
            self.remove(id, IDENTITY_KEY_SUFFIX)
        }

        fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError> {
            self.remove(id, DB_KEY_SUFFIX)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use keyring_core::mock;

    use super::testing::FakeKeychain;
    use super::*;

    const PATTERN: [u8; 32] = [0x5A; 32];

    fn mock_keychain() -> OsKeychain {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            keyring_core::set_default_store(mock::Store::new().expect("mock store"));
        });
        OsKeychain::native().expect("mock store")
    }

    #[test]
    fn store_then_load_round_trips() {
        let keychain = mock_keychain();
        let id = AccountId::new();
        keychain
            .store_identity_key(&id, &SecretKeyBytes::from_bytes(PATTERN))
            .unwrap();
        keychain
            .store_db_key(&id, &DbKey::from_bytes([0x11; 32]))
            .unwrap();

        assert_eq!(
            keychain.load_identity_key(&id).unwrap().as_bytes(),
            &PATTERN
        );
        assert_eq!(keychain.load_db_key(&id).unwrap().as_bytes(), &[0x11; 32]);
    }

    #[test]
    fn load_missing_is_not_found() {
        let keychain = mock_keychain();
        let id = AccountId::new();
        assert!(matches!(
            keychain.load_identity_key(&id),
            Err(KeychainError::NotFound)
        ));
        assert!(matches!(
            keychain.load_db_key(&id),
            Err(KeychainError::NotFound)
        ));
    }

    #[test]
    fn delete_removes_entry() {
        let keychain = mock_keychain();
        let id = AccountId::new();
        keychain
            .store_db_key(&id, &DbKey::from_bytes(PATTERN))
            .unwrap();
        keychain.delete_db_key(&id).unwrap();
        assert!(matches!(
            keychain.load_db_key(&id),
            Err(KeychainError::NotFound)
        ));
    }

    #[test]
    fn wrong_length_secret_is_a_backend_error() {
        let keychain = mock_keychain();
        let id = AccountId::new();
        keychain
            .entry(&id, DB_KEY_SUFFIX)
            .unwrap()
            .set_secret(&[1, 2, 3])
            .unwrap();
        assert!(matches!(
            keychain.load_db_key(&id),
            Err(KeychainError::Backend(_))
        ));
    }

    #[test]
    fn naming_scheme_is_per_account() {
        let (a, b) = (AccountId::new(), AccountId::new());
        assert_ne!(
            account_name(&a, DB_KEY_SUFFIX),
            account_name(&b, DB_KEY_SUFFIX)
        );
        assert_ne!(
            account_name(&a, DB_KEY_SUFFIX),
            account_name(&a, IDENTITY_KEY_SUFFIX)
        );
    }

    #[test]
    fn backend_denial_maps_to_access_denied() {
        let denied = keyring_core::Error::NoStorageAccess("locked".into());
        assert!(matches!(
            KeychainError::from(denied),
            KeychainError::AccessDenied
        ));
        assert!(matches!(
            KeychainError::from(keyring_core::Error::NoEntry),
            KeychainError::NotFound
        ));
    }

    #[test]
    fn injected_access_denied_propagates_without_a_key() {
        let keychain = FakeKeychain::new();
        let id = AccountId::new();
        keychain.fail_with(|| KeychainError::AccessDenied);
        assert!(matches!(
            keychain.load_db_key(&id),
            Err(KeychainError::AccessDenied)
        ));
        assert!(matches!(
            keychain.store_db_key(&id, &DbKey::from_bytes(PATTERN)),
            Err(KeychainError::AccessDenied)
        ));
    }

    #[test]
    fn fake_round_trips_and_deletes() {
        let keychain = FakeKeychain::new();
        let id = AccountId::new();
        keychain
            .store_identity_key(&id, &SecretKeyBytes::from_bytes(PATTERN))
            .unwrap();
        assert_eq!(
            keychain.load_identity_key(&id).unwrap().as_bytes(),
            &PATTERN
        );
        keychain.delete_identity_key(&id).unwrap();
        assert!(matches!(
            keychain.load_identity_key(&id),
            Err(KeychainError::NotFound)
        ));
    }
}
