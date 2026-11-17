use super::*;
use crate::identity::testing::FakeKeychain;
use crate::identity::{DbKey, KeychainStore, MemorySettingsStore};

fn name(raw: &str) -> DisplayName {
    validate_display_name(raw).unwrap()
}

fn sql_store() -> (tempfile::TempDir, Arc<Storage>, SqlCipherSettingsStore) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), Arc::new(FakeKeychain::new())));
    let store = SqlCipherSettingsStore::new(storage.clone());
    (dir, storage, store)
}

/// A store over a keychain that already holds a DB key for each account used.
fn with_keys(accounts: &[AccountId]) -> (tempfile::TempDir, Arc<Storage>, SqlCipherSettingsStore) {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::new());
    for a in accounts {
        keychain
            .store_db_key(a, &DbKey::from_bytes(rand::random()))
            .unwrap();
    }
    let storage = Arc::new(Storage::new(dir.path(), keychain));
    let store = SqlCipherSettingsStore::new(storage.clone());
    (dir, storage, store)
}

/// The behaviour every [`SettingsStore`] must have. Accounts are fresh per
/// call; the SQL store opens a single account per `Storage`, so the
/// account-isolation checks live in `core/tests/integration/storage_namespacing.rs`.
pub(crate) fn settings_contract(store: &dyn SettingsStore, id: &AccountId) {
    assert!(store.get_display_name(id).unwrap().is_none());
    assert!(store.get_provisioned_at(id).unwrap().is_none());
    assert_eq!(store.get_presence(id).unwrap(), None);
    assert_eq!(store.get_relay_url(id).unwrap(), None);

    store.set_display_name(id, &name("Ada")).unwrap();
    assert_eq!(store.get_display_name(id).unwrap().unwrap().as_str(), "Ada");
    store.set_display_name(id, &name("Grace")).unwrap();
    assert_eq!(
        store.get_display_name(id).unwrap().unwrap().as_str(),
        "Grace"
    );
    let unicode = "Ada 🦀 Łukasz ñ 日本語";
    store.set_display_name(id, &name(unicode)).unwrap();
    assert_eq!(
        store.get_display_name(id).unwrap().unwrap().as_str(),
        unicode
    );

    store.set_provisioned_at(id, 1_700_000_000_000).unwrap();
    assert_eq!(
        store.get_provisioned_at(id).unwrap(),
        Some(1_700_000_000_000)
    );

    store.set_presence(id, false).unwrap();
    assert_eq!(store.get_presence(id).unwrap(), Some(false));
    store.set_presence(id, true).unwrap();
    assert_eq!(store.get_presence(id).unwrap(), Some(true));

    store
        .set_relay_url(id, Some("https://relay.example"))
        .unwrap();
    assert_eq!(
        store.get_relay_url(id).unwrap().as_deref(),
        Some("https://relay.example/")
    );
    // Setters never disturb the other columns.
    assert_eq!(
        store.get_provisioned_at(id).unwrap(),
        Some(1_700_000_000_000)
    );
    store.set_relay_url(id, None).unwrap();
    assert_eq!(store.get_relay_url(id).unwrap(), None);

    let too_long = format!("https://{}.example", "a".repeat(2100));
    for bad in [
        "http://relay.example",
        "ftp://relay.example",
        "not a url",
        "",
        "http://192.168.1.5",
        too_long.as_str(),
    ] {
        assert!(
            matches!(
                store.set_relay_url(id, Some(bad)),
                Err(SettingsError::Unavailable)
            ),
            "should reject {bad:?}"
        );
    }
    for ok in [
        "http://localhost:3340",
        "http://127.0.0.1:3340",
        "http://[::1]:3340",
    ] {
        store.set_relay_url(id, Some(ok)).unwrap();
    }
    assert!(store.get_relay_url(id).unwrap().is_some());
}

#[test]
fn memory_store_meets_the_contract() {
    settings_contract(&MemorySettingsStore::new(), &AccountId::new());
}

#[test]
fn memory_accounts_are_isolated() {
    let store = MemorySettingsStore::new();
    let (a, b) = (AccountId::new(), AccountId::new());
    store.set_display_name(&a, &name("Ada")).unwrap();
    store.set_presence(&a, false).unwrap();
    assert!(store.get_display_name(&b).unwrap().is_none());
    assert_eq!(store.get_presence(&b).unwrap(), None);
}

#[test]
fn sql_store_meets_the_contract() {
    let id = AccountId::new();
    let (_dir, _storage, store) = with_keys(&[id]);
    settings_contract(&store, &id);
}

#[test]
fn sql_store_persists_across_instances() {
    let (id, keychain) = (AccountId::new(), Arc::new(FakeKeychain::new()));
    keychain
        .store_db_key(&id, &DbKey::from_bytes(rand::random()))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    {
        let storage = Arc::new(Storage::new(dir.path(), keychain.clone()));
        let store = SqlCipherSettingsStore::new(storage);
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_provisioned_at(&id, 42).unwrap();
        store.set_presence(&id, false).unwrap();
    }
    crate::identity::write_marker(dir.path(), id).unwrap();
    let storage = Arc::new(Storage::new(dir.path(), keychain));
    let reopened = SqlCipherSettingsStore::new(storage);
    assert_eq!(
        reopened.get_display_name(&id).unwrap().unwrap().as_str(),
        "Ada"
    );
    assert_eq!(reopened.get_provisioned_at(&id).unwrap(), Some(42));
    assert_eq!(reopened.get_presence(&id).unwrap(), Some(false));
}

#[test]
fn missing_key_is_unavailable() {
    let (_dir, _storage, store) = sql_store();
    assert!(matches!(
        store.get_display_name(&AccountId::new()),
        Err(SettingsError::Unavailable)
    ));
}

fn raw_update(storage: &Storage, id: &AccountId, column: &str, value: &str) {
    let db = storage.database(id).unwrap();
    // Test-only: a hostile or legacy value written behind the store's back.
    let sql = format!("UPDATE account_settings SET {column} = ?2 WHERE account_id = ?1");
    db.conn()
        .unwrap()
        .execute(&sql, params![id.as_uuid().as_bytes().as_slice(), value])
        .unwrap();
}

#[test]
fn invisible_stored_name_reads_as_unset_and_bad_name_is_unavailable() {
    let id = AccountId::new();
    let (_dir, storage, store) = with_keys(&[id]);
    store.set_display_name(&id, &name("Ada")).unwrap();
    store.set_presence(&id, false).unwrap();
    raw_update(&storage, &id, "display_name", "Ada\u{200B}");
    assert!(store.get_display_name(&id).unwrap().is_none());
    assert_eq!(store.get_presence(&id).unwrap(), Some(false));
    for bad in ["Ada\nEve", "Ada\u{202E}Eve", ""] {
        raw_update(&storage, &id, "display_name", bad);
        assert!(
            matches!(store.get_display_name(&id), Err(SettingsError::Unavailable)),
            "{bad:?}"
        );
    }
}

#[test]
fn stored_invalid_relay_reads_as_error() {
    let id = AccountId::new();
    let (_dir, storage, store) = with_keys(&[id]);
    store.set_provisioned_at(&id, 1).unwrap();
    raw_update(&storage, &id, "relay_url", "http://evil.example");
    assert!(matches!(
        store.get_relay_url(&id),
        Err(SettingsError::Unavailable)
    ));
}

#[test]
fn hostile_strings_round_trip() {
    let id = AccountId::new();
    let (_dir, _storage, store) = with_keys(&[id]);
    let hostile = "'); DROP TABLE account_settings;--\" \\ %_ ünï";
    store.set_display_name(&id, &name(hostile)).unwrap();
    assert_eq!(
        store.get_display_name(&id).unwrap().unwrap().as_str(),
        hostile
    );
    store.set_provisioned_at(&id, 3).unwrap();
    assert_eq!(store.get_provisioned_at(&id).unwrap(), Some(3));
}
