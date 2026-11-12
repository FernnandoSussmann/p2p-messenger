//! Namespacing: settings, contacts and messages written under one account are
//! invisible to another account on the same encrypted file, and the reverse.
//! Two `Storage` handles share one directory and one key, one per account.

use std::sync::Arc;

use core::contacts::{ContactStore, Nickname, SqlCipherContactStore, TrustState};
use core::identity::{
    validate_display_name, AccountId, DbKey, KeychainError, NodeId, SettingsStore,
    SqlCipherSettingsStore,
};
use core::messaging::{MessageStatus, MessageStore, SqlCipherMessageStore};
use core::storage::{KeySource, Storage};
use protocol::{new_message_id, ChatMessage};

struct FixedKey([u8; 32]);

impl KeySource for FixedKey {
    fn db_key(&self, _: &AccountId) -> Result<DbKey, KeychainError> {
        Ok(DbKey::from_bytes(self.0))
    }
}

struct Side {
    account: AccountId,
    settings: SqlCipherSettingsStore,
    contacts: SqlCipherContactStore,
    messages: SqlCipherMessageStore,
}

fn side(dir: &tempfile::TempDir, keys: &Arc<FixedKey>) -> Side {
    let account = AccountId::new();
    let storage = Arc::new(Storage::new(dir.path(), keys.clone()));
    let messages = SqlCipherMessageStore::new(storage.database(&account).unwrap());
    Side {
        account,
        settings: SqlCipherSettingsStore::new(storage.clone()),
        contacts: SqlCipherContactStore::new(storage.clone()),
        messages,
    }
}

fn peer(seed: u8) -> NodeId {
    iroh::SecretKey::from_bytes(&[seed; 32]).public()
}

fn write_all(s: &Side, tag: &str) {
    s.settings
        .set_display_name(&s.account, &validate_display_name(tag).unwrap())
        .unwrap();
    s.settings.set_provisioned_at(&s.account, 5).unwrap();
    s.settings.set_presence(&s.account, false).unwrap();
    s.settings
        .set_relay_url(&s.account, Some("https://relay.example"))
        .unwrap();
    let contact = core::contacts::Contact {
        node_id: peer(1),
        nickname: Nickname::parse(tag).unwrap(),
        self_name: None,
        trust: TrustState::Unverified,
        added_at_ms: 1,
        last_seen_ms: None,
    };
    s.contacts.insert(&s.account, contact).unwrap();
    let msg = ChatMessage {
        id: new_message_id(),
        sent_at_ms: 1,
        body: format!("secret {tag}"),
    };
    s.messages
        .insert_outgoing(&s.account, &peer(1), &msg, MessageStatus::Pending)
        .unwrap();
}

fn assert_sees_nothing(reader: &Side, writer_tag: &str) {
    let id = &reader.account;
    assert!(reader.settings.get_display_name(id).unwrap().is_none());
    assert_eq!(reader.settings.get_provisioned_at(id).unwrap(), None);
    assert_eq!(reader.settings.get_presence(id).unwrap(), None);
    assert_eq!(reader.settings.get_relay_url(id).unwrap(), None);
    assert!(reader.contacts.list(id).unwrap().is_empty());
    assert!(reader.contacts.get(id, &peer(1)).unwrap().is_none());
    assert!(reader
        .messages
        .page(id, &peer(1), None, 10)
        .unwrap()
        .is_empty());
    assert!(reader
        .messages
        .search(id, None, writer_tag, 10)
        .unwrap()
        .is_empty());
    assert!(reader.messages.undelivered(id, 10).unwrap().is_empty());
}

fn check(writer_first: bool) {
    let dir = tempfile::tempdir().unwrap();
    let keys = Arc::new(FixedKey(rand::random()));
    let (a, b) = (side(&dir, &keys), side(&dir, &keys));
    let (writer, reader) = if writer_first { (&a, &b) } else { (&b, &a) };
    write_all(writer, "Alpha");
    assert_sees_nothing(reader, "Alpha");
    // The writer still sees its own data.
    assert_eq!(
        writer.contacts.list(&writer.account).unwrap().len(),
        1,
        "own contacts"
    );
    // Once the reader has data of its own, neither side sees the other's.
    write_all(reader, "Bravo");
    assert!(writer
        .messages
        .search(&writer.account, None, "Bravo", 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        reader
            .contacts
            .get(&reader.account, &peer(1))
            .unwrap()
            .unwrap()
            .nickname
            .as_str(),
        "Bravo"
    );
    assert_eq!(
        writer
            .settings
            .get_display_name(&writer.account)
            .unwrap()
            .unwrap()
            .as_str(),
        "Alpha"
    );
}

#[test]
fn b_sees_nothing_of_a() {
    check(true);
}

#[test]
fn a_sees_nothing_of_b() {
    check(false);
}
