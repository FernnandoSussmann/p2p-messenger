//! The message store opened through the public API on a provisioned identity.

use std::sync::Arc;

use core::identity::testing::FakeKeychain;
use core::identity::{IdentityService, KeychainStore, MemorySettingsStore, NodeIdTicketProvider};
use core::messaging::{
    open_message_store, HistoryCursor, MessageStatus, MessageStore, MessageStoreError, DB_FILE,
};
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
    let identity = service(&keychain, &dir);
    let provisioned = identity.provision().unwrap();
    let peer = iroh::SecretKey::generate().public();
    let path = dir.path().join(DB_FILE);
    assert!(!path.exists());

    let (account, sent) = {
        let (account, store) = open_message_store(&identity).unwrap();
        assert_eq!(account, provisioned.account_id);
        assert!(path.exists());
        let sent = store
            .insert_outgoing(
                &account,
                &peer,
                &chat("hello 👋\nthere"),
                MessageStatus::Pending,
            )
            .unwrap();
        (account, sent)
    };

    let (again, store) = open_message_store(&identity).unwrap();
    assert_eq!(again, account);
    let rows = store.page(&account, &peer, None, 10).unwrap();
    assert_eq!(rows, vec![sent.clone()]);
    assert_eq!(HistoryCursor::of(&rows[0]).seq, sent.seq);
    assert_eq!(store.undelivered(&account, 10).unwrap(), vec![sent]);
}

#[test]
fn missing_db_key_fails_open_and_keeps_the_file() {
    let (keychain, dir) = (Arc::new(FakeKeychain::new()), tempfile::tempdir().unwrap());
    let identity = service(&keychain, &dir);
    let provisioned = identity.provision().unwrap();
    open_message_store(&identity).unwrap();
    let path = dir.path().join(DB_FILE);
    let before = std::fs::read(&path).unwrap();

    keychain.delete_db_key(&provisioned.account_id).unwrap();
    let err = open_message_store(&identity).err().unwrap();
    assert_eq!(err, MessageStoreError::Unavailable);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
