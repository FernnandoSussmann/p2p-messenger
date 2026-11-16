//! DD-05 migration forward for the settings and contacts tables, and the
//! one-shot import of the pre-DD-05 plaintext files, through the public API.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use core::contacts::{ContactStore, ContactStoreError, SqlCipherContactStore, TrustState};
use core::identity::testing::FakeKeychain;
use core::identity::{
    write_marker, AccountId, DbKey, KeychainStore, SettingsStore, SqlCipherSettingsStore,
};
use core::messaging::{MessageStore, SqlCipherMessageStore};
use core::storage::{Storage, StorageError, StorageStatus, DB_FILE, SCHEMA_VERSION};

const CANARY_NICK: &str = "CanaryNick7f3a";
const CANARY_NAME: &str = "CanaryName91bc";

/// The DD-04 schema exactly as it shipped, with `user_version = 1`.
const DD04_SCHEMA: &str = "
CREATE TABLE messages (
  seq            INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id     BLOB    NOT NULL CHECK (length(account_id) = 16),
  peer           TEXT    NOT NULL,
  message_id     BLOB    NOT NULL CHECK (length(message_id) = 16),
  direction      TEXT    NOT NULL CHECK (direction IN ('in','out')),
  sent_at_ms     INTEGER NOT NULL,
  received_at_ms INTEGER,
  body           TEXT    NOT NULL,
  status         TEXT    NOT NULL,
  UNIQUE (account_id, peer, message_id)
);
CREATE INDEX messages_by_peer_time ON messages (account_id, peer, sent_at_ms, seq);
CREATE INDEX messages_outbox       ON messages (account_id, status, seq) WHERE direction = 'out';
CREATE VIRTUAL TABLE messages_fts USING fts5 (body, content = 'messages', content_rowid = 'seq', tokenize = 'trigram');
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts (rowid, body) VALUES (new.seq, new.body); END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.seq, old.body); END;
";

struct Install {
    dir: tempfile::TempDir,
    keychain: Arc<FakeKeychain>,
    account: AccountId,
    key: [u8; 32],
}

fn node(seed: u8) -> core::identity::NodeId {
    iroh::SecretKey::from_bytes(&[seed; 32]).public()
}

impl Install {
    /// An installed identity (marker and DB key) with no files yet.
    fn new() -> Self {
        let (dir, keychain, account) = (
            tempfile::tempdir().unwrap(),
            Arc::new(FakeKeychain::new()),
            AccountId::new(),
        );
        let key: [u8; 32] = rand::random();
        keychain
            .store_db_key(&account, &DbKey::from_bytes(key))
            .unwrap();
        write_marker(dir.path(), account).unwrap();
        Self {
            dir,
            keychain,
            account,
            key,
        }
    }

    fn storage(&self) -> Arc<Storage> {
        Arc::new(Storage::new(self.dir.path(), self.keychain.clone()))
    }

    fn path(&self, file: &str) -> std::path::PathBuf {
        self.dir.path().join(file)
    }

    fn write(&self, file: &str, body: &serde_json::Value) {
        fs::write(self.path(file), body.to_string()).unwrap();
    }

    fn acct(&self) -> String {
        self.account.as_uuid().hyphenated().to_string()
    }

    fn settings_doc(&self) -> serde_json::Value {
        serde_json::json!({"version": 1, "accounts": {self.acct(): {
            "display_name": CANARY_NAME, "provisioned_at": 1_700_000_000_000i64,
            "presence_enabled": false, "relay_url": "https://relay.example"
        }}})
    }

    fn contacts_doc(&self) -> serde_json::Value {
        serde_json::json!({"version": 1, "accounts": {self.acct(): {
            node(1).to_string(): {"nickname": CANARY_NICK, "self_name": "Robert",
                "trust": "verified", "added_at_ms": 11, "last_seen_ms": 22}
        }}})
    }

    fn known_peers_doc(&self) -> serde_json::Value {
        serde_json::json!({"version": 1, "accounts": {self.acct(): {
            node(2).to_string(): {"name": "Cy", "added_at_ms": 33},
            node(1).to_string(): {"name": "Clobber", "added_at_ms": 44}
        }}})
    }

    /// A DD-04-era database: schema v1 with two messages.
    fn write_dd04_db(&self) -> iroh::EndpointId {
        let conn = rusqlite::Connection::open(self.path(DB_FILE)).unwrap();
        let hex: String = self.key.iter().map(|b| format!("{b:02x}")).collect();
        conn.execute_batch(&["PRAGMA key = \"x'", &hex, "'\";"].concat())
            .unwrap();
        conn.execute_batch(DD04_SCHEMA).unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
        let peer = node(9);
        for n in 0..2u8 {
            conn.execute(
                "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, ?5, 'pending')",
                rusqlite::params![
                    self.account.as_uuid().as_bytes().as_slice(),
                    peer.to_string(),
                    uuid::Uuid::new_v4().as_bytes().as_slice(),
                    i64::from(n),
                    format!("message {n}")
                ],
            )
            .unwrap();
        }
        peer
    }

    fn user_version(&self) -> u32 {
        let conn = rusqlite::Connection::open(self.path(DB_FILE)).unwrap();
        let hex: String = self.key.iter().map(|b| format!("{b:02x}")).collect();
        conn.execute_batch(&["PRAGMA key = \"x'", &hex, "'\";"].concat())
            .unwrap();
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }
}

fn scan_for(dir: &Path, needles: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
            hits.extend(
                needles
                    .iter()
                    .filter(|n| text.contains(**n))
                    .map(|n| format!("{n} in {path:?}")),
            );
        }
    }
    hits
}

#[test]
fn dd04_profile_is_migrated_and_imported() {
    let i = Install::new();
    let peer = i.write_dd04_db();
    i.write("settings.json", &i.settings_doc());
    i.write("contacts.json", &i.contacts_doc());
    i.write("known_peers.json", &i.known_peers_doc());
    fs::write(i.path("settings.json.tmp"), b"leftover").unwrap();

    let storage = i.storage();
    let db = storage.database(&i.account).unwrap();
    assert_eq!(storage.status(), StorageStatus::Ready);

    let messages = SqlCipherMessageStore::new(db);
    let rows = messages.page(&i.account, &peer, None, 10).unwrap();
    assert_eq!(rows.len(), 2);

    let settings = SqlCipherSettingsStore::new(storage.clone());
    assert_eq!(
        settings
            .get_display_name(&i.account)
            .unwrap()
            .unwrap()
            .as_str(),
        CANARY_NAME
    );
    assert_eq!(
        settings.get_provisioned_at(&i.account).unwrap(),
        Some(1_700_000_000_000)
    );
    assert_eq!(settings.get_presence(&i.account).unwrap(), Some(false));
    assert_eq!(
        settings.get_relay_url(&i.account).unwrap().as_deref(),
        Some("https://relay.example/")
    );

    let contacts = SqlCipherContactStore::new(storage.clone());
    let bob = contacts.get(&i.account, &node(1)).unwrap().unwrap();
    assert_eq!(bob.nickname.as_str(), CANARY_NICK);
    assert_eq!(bob.self_name.as_ref().unwrap().as_str(), "Robert");
    assert_eq!(bob.trust, TrustState::Verified);
    assert_eq!((bob.added_at_ms, bob.last_seen_ms), (11, Some(22)));
    // known_peers.json adds only what contacts.json lacks, always unverified.
    let cy = contacts.get(&i.account, &node(2)).unwrap().unwrap();
    assert_eq!(cy.trust, TrustState::Unverified);
    assert_eq!(cy.nickname.as_str(), "Cy");
    assert_eq!(contacts.list(&i.account).unwrap().len(), 2);

    for f in [
        "settings.json",
        "contacts.json",
        "known_peers.json",
        "settings.json.tmp",
    ] {
        assert!(!i.path(f).exists(), "{f} must be gone");
    }
    assert_eq!(
        scan_for(i.dir.path(), &[CANARY_NICK, CANARY_NAME]),
        Vec::<String>::new()
    );
    drop((messages, settings, contacts, storage));
    assert_eq!(i.user_version(), SCHEMA_VERSION);

    // A second open does no work and changes nothing.
    let again = i.storage();
    let contacts = SqlCipherContactStore::new(again);
    assert_eq!(contacts.list(&i.account).unwrap().len(), 2);
}

#[test]
fn stale_legacy_file_is_never_reimported() {
    let i = Install::new();
    i.write("contacts.json", &i.contacts_doc());
    {
        let contacts = SqlCipherContactStore::new(i.storage());
        assert!(contacts.get(&i.account, &node(1)).unwrap().is_some());
        // The user revokes the contact.
        assert!(contacts.remove(&i.account, &node(1)).unwrap());
    }
    // The unlink had failed and the file came back.
    i.write("contacts.json", &i.contacts_doc());
    let contacts = SqlCipherContactStore::new(i.storage());
    assert!(contacts.get(&i.account, &node(1)).unwrap().is_none());
    assert!(!i.path("contacts.json").exists(), "removal is retried");
}

#[test]
fn corrupt_settings_file_blocks_with_typed_error() {
    corrupt_file_blocks("settings.json");
}

#[test]
fn corrupt_contacts_file_blocks_with_typed_error() {
    corrupt_file_blocks("contacts.json");
}

fn corrupt_file_blocks(file: &str) {
    let i = Install::new();
    let bad: &[u8] = b"{ not json \xff";
    fs::write(i.path(file), bad).unwrap();

    let storage = i.storage();
    assert_eq!(
        storage.database(&i.account).err(),
        Some(StorageError::LegacyImportFailed)
    );
    assert_eq!(
        storage.status(),
        StorageStatus::Failed(StorageError::LegacyImportFailed)
    );
    assert_eq!(fs::read(i.path(file)).unwrap(), bad, "{file} untouched");

    // The failure is reproduced on the next start.
    let reloaded = i.storage();
    assert_eq!(
        reloaded.database(&i.account).err(),
        Some(StorageError::LegacyImportFailed)
    );
    assert_eq!(fs::read(i.path(file)).unwrap(), bad);

    // The user fixes the file and retries.
    fs::remove_file(i.path(file)).unwrap();
    storage.retry();
    storage.database(&i.account).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_legacy_file_is_typed_error_and_target_untouched() {
    let i = Install::new();
    let target = i.path("elsewhere.json");
    fs::write(&target, i.contacts_doc().to_string()).unwrap();
    let before = fs::read(&target).unwrap();
    std::os::unix::fs::symlink(&target, i.path("contacts.json")).unwrap();

    assert_eq!(
        i.storage().database(&i.account).err(),
        Some(StorageError::LegacyImportFailed)
    );
    assert_eq!(fs::read(&target).unwrap(), before);
    assert!(fs::symlink_metadata(i.path("contacts.json"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn symlinked_known_peers_is_skipped_and_untouched() {
    let i = Install::new();
    i.write("settings.json", &i.settings_doc());
    let target = i.path("elsewhere.json");
    fs::write(&target, i.known_peers_doc().to_string()).unwrap();
    let before = fs::read(&target).unwrap();
    std::os::unix::fs::symlink(&target, i.path("known_peers.json")).unwrap();

    let storage = i.storage();
    storage.database(&i.account).unwrap();
    assert_eq!(fs::read(&target).unwrap(), before);
    assert!(fs::symlink_metadata(i.path("known_peers.json"))
        .unwrap()
        .file_type()
        .is_symlink());
    let contacts = SqlCipherContactStore::new(storage);
    assert!(contacts.list(&i.account).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn hardlinked_legacy_file_is_unlinked_not_zeroed() {
    let i = Install::new();
    i.write("contacts.json", &i.contacts_doc());
    let other_link = i.path("backup.json");
    fs::hard_link(i.path("contacts.json"), &other_link).unwrap();
    let before = fs::read(&other_link).unwrap();

    i.storage().database(&i.account).unwrap();
    assert!(!i.path("contacts.json").exists());
    assert_eq!(fs::read(&other_link).unwrap(), before);
}

#[test]
fn corrupt_known_peers_is_skipped_and_left_on_disk() {
    let i = Install::new();
    i.write("settings.json", &i.settings_doc());
    i.write("contacts.json", &i.contacts_doc());
    let bad: &[u8] = b"{ not json \xff";
    fs::write(i.path("known_peers.json"), bad).unwrap();

    let storage = i.storage();
    storage.database(&i.account).unwrap();
    assert_eq!(fs::read(i.path("known_peers.json")).unwrap(), bad);
    assert!(!i.path("settings.json").exists());
    assert!(!i.path("contacts.json").exists());
    let contacts = SqlCipherContactStore::new(storage);
    assert_eq!(contacts.list(&i.account).unwrap().len(), 1);
}

#[test]
fn relay_override_survives_import() {
    let i = Install::new();
    i.write("settings.json", &i.settings_doc());
    let settings = SqlCipherSettingsStore::new(i.storage());
    assert_eq!(
        settings.get_relay_url(&i.account).unwrap().as_deref(),
        Some("https://relay.example/")
    );

    // A tampered relay is not silently dropped.
    let j = Install::new();
    let mut doc = j.settings_doc();
    doc["accounts"][j.acct()]["relay_url"] = "http://evil.example".into();
    j.write("settings.json", &doc);
    assert_eq!(
        j.storage().database(&j.account).err(),
        Some(StorageError::LegacyImportFailed)
    );
    assert!(j.path("settings.json").exists());
}

#[test]
fn legacy_trust_is_carried_and_known_peers_are_unverified() {
    let i = Install::new();
    i.write("contacts.json", &i.contacts_doc());
    i.write("known_peers.json", &i.known_peers_doc());
    let contacts = SqlCipherContactStore::new(i.storage());
    assert_eq!(
        contacts.get(&i.account, &node(1)).unwrap().unwrap().trust,
        TrustState::Verified
    );
    assert_eq!(
        contacts.get(&i.account, &node(2)).unwrap().unwrap().trust,
        TrustState::Unverified
    );
}

#[test]
fn nothing_is_created_when_there_is_nothing_to_import() {
    let i = Install::new();
    let contacts = SqlCipherContactStore::new(i.storage());
    assert!(matches!(
        contacts.get(&i.account, &node(1)),
        Err(ContactStoreError::Unavailable)
    ));
    assert!(
        !i.path(DB_FILE).exists(),
        "marker present, DB absent: Missing"
    );
}
