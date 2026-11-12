use std::fs;
use std::sync::Arc;
use std::time::Duration;

use messenger_core::contacts::testing::{MemoryContactStore, RecordingHistoryEraser};
use messenger_core::discovery::{LiveTicketProvider, NodeConfig};
use messenger_core::identity::testing::FakeKeychain;
use messenger_core::identity::{
    FileSettingsStore, KeychainStore, NodeIdTicketProvider, SettingsStore,
};
use messenger_core::messaging::{MessageStatus, MessageStore};
use messenger_core::storage::DB_FILE;
use protocol::{new_message_id, ChatMessage};

use super::*;
use crate::contacts_cmd::ContactsState;
use crate::net_cmd::{start_if_present, NetState};
use crate::test_util::assert_forms_absent;

type Service = IdentityService<Arc<FakeKeychain>, FileSettingsStore, NodeIdTicketProvider>;

/// The first-run wiring: the file settings store writes `settings.json`, which
/// is what allows the first open to create the database.
fn setup() -> (tempfile::TempDir, Arc<FakeKeychain>, Service) {
    let (dir, keychain) = (tempfile::tempdir().unwrap(), Arc::new(FakeKeychain::new()));
    let service = IdentityService::new(
        keychain.clone(),
        FileSettingsStore::new(dir.path()),
        NodeIdTicketProvider,
        dir.path(),
    );
    (dir, keychain, service)
}

fn fresh(dir: &tempfile::TempDir, keychain: &Arc<FakeKeychain>) -> (Storage, ChatState) {
    (Storage::new(dir.path(), keychain.clone()), ChatState::new())
}

fn error_kind(dto: &StorageStatusDto) -> Option<StorageErrorKind> {
    match dto {
        StorageStatusDto::Error { kind, .. } => Some(*kind),
        _ => None,
    }
}

#[test]
fn status_is_not_provisioned_without_identity() {
    let (dir, keychain, service) = setup();
    let (storage, chat) = fresh(&dir, &keychain);
    assert_eq!(
        status(&service, &storage, &chat),
        StorageStatusDto::NotProvisioned
    );
    assert!(!dir.path().join(DB_FILE).exists());
}

#[test]
fn status_is_not_provisioned_when_identity_is_in_error() {
    let (dir, keychain, service) = setup();
    fs::write(dir.path().join("account_id"), "not-a-uuid").unwrap();
    let (storage, chat) = fresh(&dir, &keychain);
    assert_eq!(
        status(&service, &storage, &chat),
        StorageStatusDto::NotProvisioned
    );
}

#[test]
fn status_ready_after_provision() {
    let (dir, keychain, service) = setup();
    let account = service.provision().unwrap().account_id;
    let (storage, chat) = fresh(&dir, &keychain);
    assert_eq!(open_for(&storage, &chat, &account), StorageStatusDto::Ready);
    assert_eq!(status(&service, &storage, &chat), StorageStatusDto::Ready);
    assert!(dir.path().join(DB_FILE).exists());
}

fn seeded() -> (tempfile::TempDir, Arc<FakeKeychain>, Service, AccountId) {
    let (dir, keychain, service) = setup();
    let account = service.provision().unwrap().account_id;
    let (storage, chat) = fresh(&dir, &keychain);
    assert_eq!(open_for(&storage, &chat, &account), StorageStatusDto::Ready);
    let (_, store) = chat.store_ref().unwrap();
    let msg = ChatMessage {
        id: new_message_id(),
        sent_at_ms: 1,
        body: "kept".into(),
    };
    let peer = iroh::SecretKey::generate().public();
    store
        .insert_outgoing(&account, &peer, &msg, MessageStatus::Pending)
        .unwrap();
    (dir, keychain, service, account)
}

#[test]
fn wrong_key_surfaces_error_and_keeps_file() {
    let (dir, keychain, service, account) = seeded();
    let before = fs::read(dir.path().join(DB_FILE)).unwrap();
    keychain.replace_db_key_with_random(&account);
    let (storage, chat) = fresh(&dir, &keychain);
    let dto = status(&service, &storage, &chat);
    assert_eq!(error_kind(&dto), Some(StorageErrorKind::WrongKeyOrCorrupt));
    assert!(chat.store_ref().is_err());
    assert_eq!(fs::read(dir.path().join(DB_FILE)).unwrap(), before);
}

#[test]
fn missing_db_surfaces_missing() {
    let (dir, keychain, service, _) = seeded();
    fs::remove_file(dir.path().join(DB_FILE)).unwrap();
    fs::remove_file(dir.path().join("settings.json")).unwrap();
    let (storage, chat) = fresh(&dir, &keychain);
    let dto = status(&service, &storage, &chat);
    assert_eq!(error_kind(&dto), Some(StorageErrorKind::Missing));
    assert!(!dir.path().join(DB_FILE).exists());
}

#[test]
fn retry_recovers_after_key_restored() {
    let (dir, keychain, service, account) = seeded();
    let key = keychain.load_db_key(&account).unwrap();
    keychain.replace_db_key_with_random(&account);
    let (storage, chat) = fresh(&dir, &keychain);
    let dto = status(&service, &storage, &chat);
    assert_eq!(error_kind(&dto), Some(StorageErrorKind::WrongKeyOrCorrupt));
    // Latched: still failing without a retry, even with the key back.
    keychain.store_db_key(&account, &key).unwrap();
    assert_eq!(
        error_kind(&status(&service, &storage, &chat)),
        Some(StorageErrorKind::WrongKeyOrCorrupt)
    );
    assert_eq!(retry(&service, &storage, &chat), StorageStatusDto::Ready);
    assert!(chat.store_ref().is_ok());
}

#[test]
fn status_json_never_contains_db_key() {
    let (dir, keychain, service, account) = seeded();
    let key = *keychain.load_db_key(&account).unwrap().as_bytes();
    keychain.replace_db_key_with_random(&account);
    let (storage, chat) = fresh(&dir, &keychain);
    for dto in [
        status(&service, &storage, &chat),
        StorageStatusDto::Ready,
        StorageStatusDto::NotProvisioned,
    ] {
        assert_forms_absent(&serde_json::to_string(&dto).unwrap(), &key);
    }
}

#[test]
fn reasons_name_no_path_or_detail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    for err in [
        StorageError::KeyMissing,
        StorageError::Missing,
        StorageError::KeychainDenied,
        StorageError::KeychainUnavailable,
        StorageError::NotEncrypted,
        StorageError::WrongKeyOrCorrupt,
        StorageError::Corrupt,
        StorageError::NewerSchema,
        StorageError::MigrationFailed { version: 2 },
        StorageError::OtherAccount,
        StorageError::Io,
    ] {
        let json = serde_json::to_string(&StorageStatusDto::from(err)).unwrap();
        assert!(!json.contains(&path) && !json.contains(DB_FILE) && !json.contains("sqlite"));
    }
    assert_eq!(
        StorageStatusDto::from(StorageError::NotProvisioned),
        StorageStatusDto::NotProvisioned
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn start_if_present_does_not_start_node_when_storage_fails() {
    let (dir, keychain) = (tempfile::tempdir().unwrap(), Arc::new(FakeKeychain::new()));
    let tickets = LiveTicketProvider::new();
    let settings = Arc::new(FileSettingsStore::new(dir.path()));
    let service = IdentityService::new(
        keychain.clone(),
        settings.clone(),
        tickets.clone(),
        dir.path(),
    );
    let account = service.provision().unwrap().account_id;
    Storage::new(dir.path(), keychain.clone())
        .database(&account)
        .unwrap();
    let key = keychain.load_db_key(&account).unwrap();
    keychain.replace_db_key_with_random(&account);

    let (storage, chat) = fresh(&dir, &keychain);
    let state = NetState::default();
    let contacts = ContactsState::new(
        Arc::new(MemoryContactStore::new()),
        Arc::new(RecordingHistoryEraser::new()),
    );
    let start = || {
        start_if_present(
            &state,
            &service,
            settings.clone() as Arc<dyn SettingsStore + Send + Sync>,
            tickets.clone(),
            &contacts,
            &chat,
            &storage,
            NodeConfig::loopback(),
        )
    };
    assert!(!start().await);
    assert!(state.node().is_err());

    keychain.store_db_key(&account, &key).unwrap();
    storage.retry();
    assert!(start().await);
    assert!(state.node().is_ok());
    state.shutdown_with_timeout(Duration::from_secs(2)).await;
}
