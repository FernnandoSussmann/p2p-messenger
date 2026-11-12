//! Provisioning cleanup (`Storage::discard_provisioning`): a failed
//! provisioning never leaves the app stuck and never deletes data that is not
//! its own.

use std::fs;
use std::sync::Arc;

use super::*;
use crate::contacts::{Contact, ContactStore, Nickname, SqlCipherContactStore, TrustState};
use crate::identity::testing::FakeKeychain;
use crate::identity::{
    AccountId, DbKey, IdentityService, KeychainStore, NodeIdTicketProvider, ProvisionError,
    SqlCipherSettingsStore,
};

type Service = IdentityService<Arc<FakeKeychain>, SqlCipherSettingsStore, NodeIdTicketProvider>;

fn setup() -> (tempfile::TempDir, Arc<FakeKeychain>, Arc<Storage>, Service) {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::new());
    let storage = Arc::new(Storage::new(dir.path(), keychain.clone()));
    let service = IdentityService::new(
        keychain.clone(),
        SqlCipherSettingsStore::new(storage.clone()),
        NodeIdTicketProvider,
        dir.path(),
    );
    (dir, keychain, storage, service)
}

fn db_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join(DB_FILE)
}

fn with_key(keychain: &FakeKeychain, account: &AccountId) {
    keychain
        .store_db_key(account, &DbKey::from_bytes(rand::random()))
        .unwrap();
}

#[test]
fn failed_set_provisioned_at_then_reprovision_succeeds() {
    let (dir, keychain, storage, service) = setup();
    // A directory in place of the file: the open fails with `Io` and latches.
    fs::create_dir(db_path(&dir)).unwrap();
    assert!(matches!(
        service.provision(),
        Err(ProvisionError::Settings(_))
    ));
    assert!(db_path(&dir).is_dir(), "not created by this attempt");
    assert_eq!(storage.status(), StorageStatus::NotOpened);
    assert!(keychain.is_empty());

    fs::remove_dir(db_path(&dir)).unwrap();
    let provisioned = service.provision().unwrap();
    assert_eq!(storage.status(), StorageStatus::Ready);
    storage.database(&provisioned.account_id).unwrap();
}

#[test]
fn failed_marker_write_then_reprovision_succeeds() {
    let (dir, keychain, storage, service) = setup();
    // The marker is written through `account_id.tmp`; a directory there makes
    // the write fail after the database was created.
    let obstacle = dir.path().join("account_id.tmp");
    fs::create_dir(&obstacle).unwrap();
    assert!(matches!(
        service.provision(),
        Err(ProvisionError::Marker(_))
    ));
    assert!(!db_path(&dir).exists(), "the attempt's empty DB is removed");
    assert_eq!(storage.status(), StorageStatus::NotOpened);
    assert!(keychain.is_empty());

    fs::remove_dir(&obstacle).unwrap();
    let provisioned = service.provision().unwrap();
    assert_eq!(storage.status(), StorageStatus::Ready);
    storage.database(&provisioned.account_id).unwrap();
}

#[test]
fn existing_db_is_never_discarded() {
    let (dir, keychain, storage, service) = setup();
    let other = AccountId::new();
    with_key(&keychain, &other);
    Storage::new(dir.path(), keychain.clone())
        .database_creating(&other)
        .unwrap();
    let before = fs::read(db_path(&dir)).unwrap();

    // The new account's key does not open it, so provisioning fails.
    assert!(service.provision().is_err());
    assert_eq!(fs::read(db_path(&dir)).unwrap(), before);
    assert_eq!(storage.status(), StorageStatus::NotOpened);
}

#[test]
fn db_with_user_data_is_never_discarded() {
    let (dir, keychain, storage, _) = setup();
    let account = AccountId::new();
    with_key(&keychain, &account);
    storage.database_creating(&account).unwrap();
    let contact = Contact {
        node_id: iroh::SecretKey::generate().public(),
        nickname: Nickname::parse("Bob").unwrap(),
        self_name: None,
        trust: TrustState::Unverified,
        added_at_ms: 1,
        last_seen_ms: None,
    };
    SqlCipherContactStore::new(storage.clone())
        .insert(&account, contact)
        .unwrap();

    storage.discard_provisioning(&account).unwrap();
    assert!(db_path(&dir).exists());
    assert_eq!(storage.status(), StorageStatus::NotOpened);
}

#[test]
fn discard_with_extra_handle_defers_and_never_returns_other_account() {
    let (dir, keychain, storage, _) = setup();
    let account = AccountId::new();
    with_key(&keychain, &account);
    let extra = storage.database_creating(&account).unwrap();

    storage.discard_provisioning(&account).unwrap();
    assert!(db_path(&dir).exists(), "kept while another handle lives");
    assert_eq!(storage.status(), StorageStatus::NotOpened);

    for who in [account, AccountId::new()] {
        assert_eq!(storage.database(&who).err(), Some(StorageError::Io));
    }
    assert_eq!(storage.status(), StorageStatus::NotOpened, "not latched");

    drop(extra);
    let fresh = storage.database(&account).unwrap();
    assert_eq!(storage.status(), StorageStatus::Ready);
    drop(fresh);
}

#[test]
fn a_provisioned_account_is_never_discarded() {
    let (dir, _keychain, storage, service) = setup();
    let provisioned = service.provision().unwrap();
    storage
        .discard_provisioning(&provisioned.account_id)
        .unwrap();
    assert!(db_path(&dir).exists());
    assert_eq!(storage.status(), StorageStatus::NotOpened);
    storage.database(&provisioned.account_id).unwrap();
}
