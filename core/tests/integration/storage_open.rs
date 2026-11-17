//! `Storage` end to end over a fake keychain.

use std::sync::Arc;

use core::identity::testing::FakeKeychain;
use core::identity::{
    IdentityService, KeychainError, KeychainStore, MemorySettingsStore, NodeIdTicketProvider,
};
use core::storage::{Storage, StorageError, StorageStatus, DB_FILE};

type Service = IdentityService<Arc<FakeKeychain>, MemorySettingsStore, NodeIdTicketProvider>;

fn setup() -> (tempfile::TempDir, Arc<FakeKeychain>, Service) {
    let (dir, keychain) = (tempfile::tempdir().unwrap(), Arc::new(FakeKeychain::new()));
    let service = IdentityService::new(
        Arc::clone(&keychain),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        dir.path(),
    );
    (dir, keychain, service)
}

#[test]
fn key_missing_is_reported() {
    let (dir, keychain, service) = setup();
    let account = service.provision().unwrap().account_id;
    let storage = Storage::new(dir.path(), keychain.clone());
    storage.database_creating(&account).unwrap();
    drop(storage);
    let before = std::fs::read(dir.path().join(DB_FILE)).unwrap();

    keychain.delete_db_key(&account).unwrap();
    let storage = Storage::new(dir.path(), keychain);
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::KeyMissing)
    );
    assert_eq!(
        storage.status(),
        StorageStatus::Failed(StorageError::KeyMissing)
    );
    assert_eq!(std::fs::read(dir.path().join(DB_FILE)).unwrap(), before);
}

#[test]
fn keychain_denied_is_reported_and_file_kept() {
    let (dir, keychain, service) = setup();
    let account = service.provision().unwrap().account_id;
    Storage::new(dir.path(), keychain.clone())
        .database_creating(&account)
        .unwrap();
    let before = std::fs::read(dir.path().join(DB_FILE)).unwrap();

    keychain.fail_with(|| KeychainError::AccessDenied);
    let storage = Storage::new(dir.path(), keychain.clone());
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::KeychainDenied)
    );
    keychain.fail_with(|| KeychainError::Backend("boom".into()));
    storage.retry();
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::KeychainUnavailable)
    );
    assert_eq!(std::fs::read(dir.path().join(DB_FILE)).unwrap(), before);
}

#[test]
fn opens_with_the_marker_accounts_key() {
    let (dir, keychain, service) = setup();
    let account = service.provision().unwrap().account_id;
    let storage = Storage::new(dir.path(), keychain);
    assert!(!dir.path().join(DB_FILE).exists());
    // `MemorySettingsStore` writes no legacy file and the marker exists: the
    // file may not be created implicitly, only by the test-support path.
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::Missing)
    );
    storage.retry();
    storage.database_creating(&account).unwrap();
    assert!(dir.path().join(DB_FILE).exists());
    assert_eq!(storage.status(), StorageStatus::Ready);
}
