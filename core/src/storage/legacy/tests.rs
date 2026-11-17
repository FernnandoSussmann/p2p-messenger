//! Report-level checks of the legacy import. End-to-end behaviour through the
//! public API lives in `core/tests/integration/storage_legacy_import.rs`.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::{import_all, ImportReport};
use crate::contacts::{Contact, Nickname, MAX_CONTACTS_PER_ACCOUNT};
use crate::identity::testing::FakeKeychain;
use crate::identity::{write_marker, AccountId, DbKey, KeychainError, KeychainStore};
use crate::storage::database::{Create, Database};
use crate::storage::schema::MIGRATIONS;
use crate::storage::*;

fn account_key(account: &AccountId) -> String {
    account.as_uuid().hyphenated().to_string()
}

fn write(dir: &tempfile::TempDir, file: &str, body: serde_json::Value) {
    fs::write(dir.path().join(file), body.to_string()).unwrap();
}

fn node(seed: u8) -> String {
    iroh::SecretKey::from_bytes(&[seed; 32])
        .public()
        .to_string()
}

fn open_db(dir: &tempfile::TempDir, key: &DbKey) -> Database {
    Database::open(&dir.path().join(DB_FILE), key, MIGRATIONS, Create::Allowed).unwrap()
}

fn contact_count(db: &Database) -> i64 {
    db.conn()
        .unwrap()
        .query_row("SELECT count(*) FROM contacts", [], |r| r.get(0))
        .unwrap()
}

fn imported_rows(db: &Database) -> i64 {
    db.conn()
        .unwrap()
        .query_row("SELECT count(*) FROM legacy_imports", [], |r| r.get(0))
        .unwrap()
}

fn settings_json(account: &AccountId) -> serde_json::Value {
    serde_json::json!({"version": 1, "accounts": {
        account_key(account): {"display_name": "Ada", "provisioned_at": 5,
            "presence_enabled": false, "relay_url": "https://relay.example"}
    }})
}

fn contacts_json(account: &AccountId, seeds: &[u8]) -> serde_json::Value {
    let contacts: serde_json::Map<_, _> = seeds
        .iter()
        .map(|s| {
            (
                node(*s),
                serde_json::json!({"nickname": "Bob", "self_name": null,
                    "trust": "verified", "added_at_ms": 1, "last_seen_ms": null}),
            )
        })
        .collect();
    serde_json::json!({"version": 1, "accounts": {account_key(account): contacts}})
}

#[test]
fn report_counts_rows_and_a_second_run_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (account, key) = (AccountId::new(), DbKey::from_bytes(rand::random()));
    let db = open_db(&dir, &key);
    write(&dir, "settings.json", settings_json(&account));
    write(&dir, "contacts.json", contacts_json(&account, &[1, 2]));
    write(
        &dir,
        "known_peers.json",
        serde_json::json!({"version": 1, "accounts": {
            account_key(&account): {node(2): {"name": null, "added_at_ms": 1}, node(3): {"name": "Cy", "added_at_ms": 2}}
        }}),
    );
    assert_eq!(
        import_all(&db, dir.path()).unwrap(),
        ImportReport {
            settings_accounts: 1,
            contacts: 2,
            known_peers: 1,
            known_peers_skipped: false,
            removals_pending: 0,
        }
    );
    for f in ["settings.json", "contacts.json", "known_peers.json"] {
        assert!(!dir.path().join(f).exists(), "{f}");
    }
    assert_eq!(contact_count(&db), 3);
    assert_eq!(imported_rows(&db), 3);
    assert_eq!(
        import_all(&db, dir.path()).unwrap(),
        ImportReport::default()
    );
}

#[test]
fn corrupt_known_peers_is_skipped_and_left_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let (account, key) = (AccountId::new(), DbKey::from_bytes(rand::random()));
    let db = open_db(&dir, &key);
    write(&dir, "settings.json", settings_json(&account));
    write(&dir, "contacts.json", contacts_json(&account, &[1]));
    let corrupt = b"{ not json \xff";
    fs::write(dir.path().join("known_peers.json"), corrupt).unwrap();

    let report = import_all(&db, dir.path()).unwrap();
    assert!(report.known_peers_skipped);
    assert_eq!((report.settings_accounts, report.contacts), (1, 1));
    assert_eq!(
        fs::read(dir.path().join("known_peers.json")).unwrap(),
        corrupt
    );
    assert!(!dir.path().join("settings.json").exists());
    assert!(!dir.path().join("contacts.json").exists());
}

#[test]
fn failed_import_rolls_back_and_keeps_file() {
    let dir = tempfile::tempdir().unwrap();
    let (account, key) = (AccountId::new(), DbKey::from_bytes(rand::random()));
    let db = open_db(&dir, &key);
    // One row already there plus a file at the cap: the commit would exceed it.
    {
        let mut conn = db.conn().unwrap();
        let tx = conn.transaction().unwrap();
        let c = Contact {
            node_id: iroh::SecretKey::from_bytes(&[200; 32]).public(),
            nickname: Nickname::parse("Existing").unwrap(),
            self_name: None,
            trust: crate::contacts::TrustState::Unverified,
            added_at_ms: 1,
            last_seen_ms: None,
        };
        crate::contacts::insert_if_absent_tx(&tx, &account, &c).unwrap();
        tx.commit().unwrap();
    }
    let contacts: serde_json::Map<_, _> = (0..MAX_CONTACTS_PER_ACCOUNT as u32)
        .map(|i| {
            let mut seed = [7u8; 32];
            seed[..4].copy_from_slice(&i.to_le_bytes());
            (
                iroh::SecretKey::from_bytes(&seed).public().to_string(),
                serde_json::json!({"nickname": "n", "self_name": null,
                    "trust": "unverified", "added_at_ms": 0, "last_seen_ms": null}),
            )
        })
        .collect();
    write(
        &dir,
        "contacts.json",
        serde_json::json!({"version": 1, "accounts": {account_key(&account): contacts}}),
    );
    let before = fs::read(dir.path().join("contacts.json")).unwrap();

    assert_eq!(
        import_all(&db, dir.path()).err(),
        Some(StorageError::LegacyImportFailed)
    );
    assert_eq!(fs::read(dir.path().join("contacts.json")).unwrap(), before);
    assert_eq!(contact_count(&db), 1);
    assert_eq!(imported_rows(&db), 0);
}

struct Counting {
    inner: FakeKeychain,
    loads: AtomicUsize,
}

impl KeySource for Counting {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_db_key(account)
    }
}

#[test]
fn import_does_not_reenter_storage() {
    let dir = tempfile::tempdir().unwrap();
    let account = AccountId::new();
    let counting = Arc::new(Counting {
        inner: FakeKeychain::new(),
        loads: AtomicUsize::new(0),
    });
    counting
        .inner
        .store_db_key(&account, &DbKey::from_bytes(rand::random()))
        .unwrap();
    write_marker(dir.path(), account).unwrap();
    write(&dir, "settings.json", settings_json(&account));
    write(&dir, "contacts.json", contacts_json(&account, &[1]));
    write(
        &dir,
        "known_peers.json",
        serde_json::json!({"version": 1, "accounts": {}}),
    );
    let storage = Storage::new(dir.path(), counting.clone());
    // A deadlock or a nested open would hang or load the key twice.
    storage.database(&account).unwrap();
    assert_eq!(counting.loads.load(Ordering::SeqCst), 1);
    assert!(!dir.path().join("settings.json").exists());
}

#[test]
fn leftover_tmp_is_removed_when_the_main_file_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_db(&dir, &DbKey::from_bytes(rand::random()));
    for f in ["settings.json", "contacts.json", "known_peers.json"] {
        fs::write(dir.path().join(format!("{f}.tmp")), b"secret").unwrap();
    }
    assert_eq!(
        import_all(&db, dir.path()).unwrap(),
        ImportReport::default()
    );
    for f in ["settings.json", "contacts.json", "known_peers.json"] {
        assert!(!dir.path().join(format!("{f}.tmp")).exists(), "{f}");
    }
}
