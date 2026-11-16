use super::*;
use crate::contacts::contract::{
    concurrent_observers, contact, contract_suite, isolation_suite, name, node,
};
use crate::identity::testing::FakeKeychain;
use crate::identity::{DbKey, KeychainStore};

/// One `Storage` per account over one directory and one shared DB key, so two
/// accounts can be exercised against the same file.
pub(crate) fn storages(dir: &tempfile::TempDir, accounts: &[AccountId]) -> Vec<Arc<Storage>> {
    let key: [u8; 32] = rand::random();
    accounts
        .iter()
        .map(|a| {
            let keychain = Arc::new(FakeKeychain::new());
            keychain.store_db_key(a, &DbKey::from_bytes(key)).unwrap();
            Arc::new(Storage::new(dir.path(), keychain))
        })
        .collect()
}

fn store(account: AccountId) -> (tempfile::TempDir, Arc<Storage>, SqlCipherContactStore) {
    let dir = tempfile::tempdir().unwrap();
    let storage = storages(&dir, &[account]).remove(0);
    let store = SqlCipherContactStore::new(storage.clone());
    (dir, storage, store)
}

#[test]
fn contract_sql() {
    let a = AccountId::new();
    let (_dir, _storage, store) = store(a);
    contract_suite(&store, a);
}

#[test]
fn isolation_sql() {
    let (a, b) = (AccountId::new(), AccountId::new());
    let dir = tempfile::tempdir().unwrap();
    let mut s = storages(&dir, &[a, b]);
    let store_b = SqlCipherContactStore::new(s.remove(1));
    let store_a = SqlCipherContactStore::new(s.remove(0));
    isolation_suite(&store_a, a, &store_b, b);
}

#[test]
fn concurrent_observers_sql() {
    let a = AccountId::new();
    let (_dir, _storage, store) = store(a);
    concurrent_observers(&store, a);
}

#[test]
fn survives_reopen() {
    let a = AccountId::new();
    let dir = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::new());
    keychain
        .store_db_key(&a, &DbKey::from_bytes(rand::random()))
        .unwrap();
    {
        let store =
            SqlCipherContactStore::new(Arc::new(Storage::new(dir.path(), keychain.clone())));
        store.insert(&a, contact(1, "Bob")).unwrap();
    }
    crate::identity::write_marker(dir.path(), a).unwrap();
    let store = SqlCipherContactStore::new(Arc::new(Storage::new(dir.path(), keychain)));
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), contact(1, "Bob"));
}

#[test]
fn same_nickname_different_nodes_are_distinct() {
    let a = AccountId::new();
    let (_dir, _storage, store) = store(a);
    store.insert(&a, contact(1, "Ada")).unwrap();
    store.insert(&a, contact(2, "Ada")).unwrap();
    assert!(store.get(&a, &node(3)).unwrap().is_none());
    assert_eq!(store.list(&a).unwrap().len(), 2);
}

#[test]
fn observe_writes_only_when_something_material_changed() {
    let a = AccountId::new();
    let (_dir, storage, store) = store(a);
    store.insert(&a, contact(1, "Bob")).unwrap();
    let bob = name("Bob");
    store
        .observe(&a, &node(1), NameUpdate::Set(Some(&bob)), Some(10_000))
        .unwrap();
    let changes = || {
        let db = storage.database(&a).unwrap();
        let n: i64 = db
            .conn()
            .unwrap()
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        n
    };
    let base = changes();
    store
        .observe(&a, &node(1), NameUpdate::Set(Some(&bob)), Some(11_000))
        .unwrap();
    store.observe(&a, &node(1), NameUpdate::Keep, None).unwrap();
    assert_eq!(changes(), base);
    let robert = name("Robert");
    store
        .observe(&a, &node(1), NameUpdate::Set(Some(&robert)), Some(12_000))
        .unwrap();
    assert!(changes() > base);
}

fn seed_many(storage: &Storage, a: &AccountId, count: usize) {
    let db = storage.database(a).unwrap();
    let mut conn = db.conn().unwrap();
    let tx = conn.transaction().unwrap();
    for i in 0..count {
        let mut seed = [7u8; 32];
        seed[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let mut c = contact(1, "n");
        c.node_id = iroh_base::SecretKey::from_bytes(&seed).public();
        insert_if_absent_tx(&tx, a, &c).unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn cap_is_enforced_by_insert_and_insert_missing() {
    let a = AccountId::new();
    let (_dir, storage, store) = store(a);
    seed_many(&storage, &a, MAX_CONTACTS_PER_ACCOUNT - 1);
    let batch = vec![(a, contact(1, "One")), (a, contact(2, "Two"))];
    assert!(matches!(
        store.insert_missing(batch),
        Err(ContactStoreError::Full)
    ));
    assert_eq!(store.list(&a).unwrap().len(), MAX_CONTACTS_PER_ACCOUNT - 1);
    store.insert(&a, contact(1, "One")).unwrap();
    assert!(matches!(
        store.insert(&a, contact(2, "Two")),
        Err(ContactStoreError::Full)
    ));
    assert!(matches!(
        store.insert(&a, contact(1, "One")),
        Err(ContactStoreError::AlreadyExists)
    ));
}

fn raw_set(storage: &Storage, a: &AccountId, column: &str, value: rusqlite::types::Value) {
    let db = storage.database(a).unwrap();
    // Test-only: corrupt a row behind the store's back.
    let sql = format!("UPDATE contacts SET {column} = ?2 WHERE account_id = ?1");
    db.conn()
        .unwrap()
        .execute(&sql, params![account_bytes(a).as_slice(), value])
        .unwrap();
}

#[test]
fn corrupt_row_is_reported() {
    use rusqlite::types::Value::Text;
    let long = "a".repeat(129);
    let cases: [(&str, &str); 6] = [
        ("nickname", "a\nb"),
        ("nickname", " padded"),
        ("nickname", long.as_str()),
        ("self_name", "x\u{202E}y"),
        ("self_name", "x\u{200B}"),
        ("node_id", "not-a-node"),
    ];
    for (column, bad) in cases {
        let a = AccountId::new();
        let (_dir, storage, store) = store(a);
        store.insert(&a, contact(1, "Bob")).unwrap();
        // Rewriting the key column would move the row; address it explicitly.
        raw_set(&storage, &a, column, Text(bad.to_owned()));
        assert!(
            matches!(store.list(&a), Err(ContactStoreError::Corrupt)),
            "{column}={bad:?}"
        );
    }
}

#[test]
fn hostile_strings_round_trip() {
    let a = AccountId::new();
    let (_dir, _storage, store) = store(a);
    let hostile = "'); DROP TABLE contacts;--\" \\ %_ ünï 日本";
    let mut c = contact(1, hostile);
    c.self_name = Some(name(hostile));
    store.insert(&a, c.clone()).unwrap();
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), c);
    store
        .set_nickname(&a, &node(1), &Nickname::parse("\"; --").unwrap())
        .unwrap();
    assert_eq!(store.list(&a).unwrap().len(), 1);
}

#[test]
fn errors_are_opaque_when_the_store_is_unavailable() {
    let a = AccountId::new();
    let dir = tempfile::tempdir().unwrap();
    let store = SqlCipherContactStore::new(Arc::new(Storage::new(
        dir.path(),
        Arc::new(FakeKeychain::new()),
    )));
    assert!(matches!(
        store.list(&a),
        Err(ContactStoreError::Unavailable)
    ));
}
