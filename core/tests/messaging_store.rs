//! The message store opened through `Storage` on a provisioned identity.

use std::sync::Arc;

use core::identity::testing::FakeKeychain;
use core::identity::{IdentityService, KeychainStore, MemorySettingsStore, NodeIdTicketProvider};
use core::messaging::{HistoryCursor, MessageStatus, MessageStore, SqlCipherMessageStore};
use core::storage::{Storage, StorageError, DB_FILE};
use protocol::{new_message_id, ChatMessage};

type Service = IdentityService<Arc<FakeKeychain>, MemorySettingsStore, NodeIdTicketProvider>;

fn service(keychain: &Arc<FakeKeychain>, dir: &tempfile::TempDir) -> Service {
    IdentityService::new(
        Arc::clone(keychain),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        dir.path(),
    )
}

fn storage(keychain: &Arc<FakeKeychain>, dir: &tempfile::TempDir) -> Storage {
    Storage::new(dir.path(), keychain.clone())
}

fn chat(body: &str) -> ChatMessage {
    ChatMessage {
        id: new_message_id(),
        sent_at_ms: 42,
        body: body.to_owned(),
    }
}

#[test]
fn rows_survive_closing_and_reopening_the_store() {
    let (keychain, dir) = (Arc::new(FakeKeychain::new()), tempfile::tempdir().unwrap());
    let account = service(&keychain, &dir).provision().unwrap().account_id;
    let peer = iroh::SecretKey::generate().public();
    let path = dir.path().join(DB_FILE);
    assert!(!path.exists());

    let sent = {
        let store = SqlCipherMessageStore::new(
            storage(&keychain, &dir)
                .database_creating(&account)
                .unwrap(),
        );
        assert!(path.exists());
        store
            .insert_outgoing(
                &account,
                &peer,
                &chat("hello 👋\nthere"),
                MessageStatus::Pending,
            )
            .unwrap()
    };

    let store = SqlCipherMessageStore::new(storage(&keychain, &dir).database(&account).unwrap());
    let rows = store.page(&account, &peer, None, 10).unwrap();
    assert_eq!(rows, vec![sent.clone()]);
    assert_eq!(HistoryCursor::of(&rows[0]).seq, sent.seq);
    assert_eq!(store.undelivered(&account, 10).unwrap(), vec![sent]);
}

#[test]
fn missing_db_key_fails_open_and_keeps_the_file() {
    let (keychain, dir) = (Arc::new(FakeKeychain::new()), tempfile::tempdir().unwrap());
    let account = service(&keychain, &dir).provision().unwrap().account_id;
    storage(&keychain, &dir)
        .database_creating(&account)
        .unwrap();
    let path = dir.path().join(DB_FILE);
    let before = std::fs::read(&path).unwrap();

    keychain.delete_db_key(&account).unwrap();
    let err = storage(&keychain, &dir).database(&account).err().unwrap();
    assert_eq!(err, StorageError::KeyMissing);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
