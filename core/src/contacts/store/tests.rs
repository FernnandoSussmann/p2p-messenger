use std::path::Path;

use super::*;
use crate::contacts::contract::{contact, contract_suite, name, node};

fn store() -> (tempfile::TempDir, FileContactStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = FileContactStore::new(dir.path());
    (dir, store)
}

fn file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(CONTACTS_FILE_NAME)
}

fn many_nodes(count: u32) -> impl Iterator<Item = NodeId> {
    (0..count).map(|i| {
        let mut seed = [7u8; 32];
        seed[..4].copy_from_slice(&i.to_le_bytes());
        iroh_base::SecretKey::from_bytes(&seed).public()
    })
}

fn seed_file(dir: &Path, account: &AccountId, count: u32) {
    let contacts = many_nodes(count)
        .map(|id| {
            (
                id.to_string(),
                StoredContact {
                    nickname: "n".into(),
                    self_name: None,
                    trust: TrustState::Unverified,
                    added_at_ms: 0,
                    last_seen_ms: None,
                },
            )
        })
        .collect();
    let doc = Document {
        version: 1,
        accounts: BTreeMap::from([(account.as_uuid().hyphenated().to_string(), contacts)]),
    };
    fs::write(
        dir.join(CONTACTS_FILE_NAME),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
}

#[test]
fn contract_file() {
    let (_dir, store) = store();
    contract_suite(&store);
}

#[test]
fn observe_skips_write_when_nothing_material_changed() {
    let (_dir, store) = store();
    let a = AccountId::new();
    store.insert(&a, contact(1, "Bob")).unwrap();
    let bob = name("Bob");
    store
        .observe(&a, &node(1), NameUpdate::Set(Some(&bob)), Some(10_000))
        .unwrap();
    let base = store.writes();

    store
        .observe(
            &a,
            &node(1),
            NameUpdate::Set(Some(&bob)),
            Some(10_000 + 1_000),
        )
        .unwrap();
    store.observe(&a, &node(1), NameUpdate::Keep, None).unwrap();
    assert_eq!(store.writes(), base);

    let robert = name("Robert");
    store
        .observe(
            &a,
            &node(1),
            NameUpdate::Set(Some(&robert)),
            Some(10_000 + 2_000),
        )
        .unwrap();
    assert_eq!(store.writes(), base + 1);

    store
        .observe(
            &a,
            &node(1),
            NameUpdate::Set(Some(&robert)),
            Some(10_000 + LAST_SEEN_RESOLUTION_MS + 5_000),
        )
        .unwrap();
    assert_eq!(store.writes(), base + 2);
}

#[test]
fn insert_missing_is_all_or_nothing_at_the_cap() {
    let (dir, store) = store();
    let a = AccountId::new();
    seed_file(dir.path(), &a, MAX_CONTACTS_PER_ACCOUNT as u32 - 1);
    let before = fs::read(file(&dir)).unwrap();
    let batch = vec![(a, contact(1, "One")), (a, contact(2, "Two"))];
    assert!(matches!(
        store.insert_missing(batch),
        Err(ContactStoreError::Full)
    ));
    assert_eq!(store.writes(), 0);
    assert_eq!(fs::read(file(&dir)).unwrap(), before);
    assert_eq!(store.list(&a).unwrap().len(), MAX_CONTACTS_PER_ACCOUNT - 1);
    store.insert(&a, contact(1, "One")).unwrap();
    assert!(matches!(
        store.insert(&a, contact(2, "Two")),
        Err(ContactStoreError::Full)
    ));
}

#[test]
fn survives_reopen() {
    let (dir, store) = store();
    let a = AccountId::new();
    store.insert(&a, contact(1, "Bob")).unwrap();
    drop(store);
    let reopened = FileContactStore::new(dir.path());
    assert_eq!(
        reopened.get(&a, &node(1)).unwrap().unwrap(),
        contact(1, "Bob")
    );
}

#[test]
fn same_nickname_different_nodes_are_distinct() {
    let (_dir, store) = store();
    let a = AccountId::new();
    store.insert(&a, contact(1, "Ada")).unwrap();
    store.insert(&a, contact(2, "Ada")).unwrap();
    assert!(store.get(&a, &node(3)).unwrap().is_none());
    assert_eq!(store.list(&a).unwrap().len(), 2);
}

#[test]
fn corrupt_files_error_and_are_not_overwritten() {
    let acct = AccountId::new().as_uuid().hyphenated().to_string();
    let id = node(1).to_string();
    let entry = |fields: &str| {
        format!(r#"{{"version":1,"accounts":{{"{acct}":{{"{id}":{{{fields}}}}}}}}}"#)
    };
    let ok = r#""nickname":"Bob","self_name":null,"trust":"unverified","added_at_ms":1,"last_seen_ms":null"#;
    let with = |from: &str, to: &str| entry(&ok.replace(from, to));
    let long = "a".repeat(129);
    let cases = [
        "not json".to_owned(),
        r#"{"version":2,"accounts":{}}"#.to_owned(),
        r#"{"version":1,"accounts":{},"extra":1}"#.to_owned(),
        entry(&format!(r#"{ok},"extra":1"#)),
        format!(r#"{{"version":1,"accounts":{{"x":{{"{id}":{{{ok}}}}}}}}}"#),
        format!(
            r#"{{"version":1,"accounts":{{"{}":{{"{id}":{{{ok}}}}}}}}}"#,
            acct.to_uppercase()
        ),
        format!(r#"{{"version":1,"accounts":{{"{acct}":{{"nope":{{{ok}}}}}}}}}"#),
        format!(
            r#"{{"version":1,"accounts":{{"{acct}":{{"{}":{{{ok}}}}}}}}}"#,
            id.to_uppercase()
        ),
        with(r#""Bob""#, r#""a\nb""#),
        with(r#""Bob""#, "\"a\\u202Eb\""),
        with(r#""Bob""#, &format!(r#""{long}""#)),
        with(r#""Bob""#, r#"" a""#),
        with(r#""self_name":null"#, r#""self_name":"\u0007""#),
        with(r#""self_name":null"#, r#""self_name":" x""#),
        with("unverified", "trusted"),
        with(r#""added_at_ms":1"#, r#""added_at_ms":-1"#),
        with(r#""last_seen_ms":null"#, r#""last_seen_ms":-5"#),
    ];
    let a = AccountId::new();
    for body in cases {
        let dir = tempfile::tempdir().unwrap();
        fs::write(file(&dir), &body).unwrap();
        let store = FileContactStore::new(dir.path());
        assert!(
            matches!(store.list(&a), Err(ContactStoreError::Corrupt)),
            "{body}"
        );
        assert!(matches!(
            store.get(&a, &node(1)),
            Err(ContactStoreError::Corrupt)
        ));
        assert!(matches!(
            store.insert(&a, contact(2, "x")),
            Err(ContactStoreError::Corrupt)
        ));
        assert!(matches!(
            store.observe(&a, &node(1), NameUpdate::Keep, Some(1)),
            Err(ContactStoreError::Corrupt)
        ));
        assert!(matches!(
            store.remove(&a, &node(1)),
            Err(ContactStoreError::Corrupt)
        ));
        assert_eq!(fs::read_to_string(file(&dir)).unwrap(), body);
    }
}

#[cfg(unix)]
#[test]
fn symlinked_file_is_refused() {
    let (dir, store) = store();
    let target = dir.path().join("elsewhere.json");
    fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
    std::os::unix::fs::symlink(&target, file(&dir)).unwrap();
    let a = AccountId::new();
    assert!(matches!(store.list(&a), Err(ContactStoreError::Corrupt)));
    assert!(store.insert(&a, contact(1, "Bob")).is_err());
}

#[test]
fn oversized_file_is_refused() {
    let (dir, store) = store();
    fs::write(file(&dir), vec![b' '; MAX_CONTACTS_BYTES as usize + 1]).unwrap();
    assert!(matches!(
        store.list(&AccountId::new()),
        Err(ContactStoreError::Corrupt)
    ));
}

#[test]
fn full_store_reports_full_and_stays_readable() {
    let (dir, store) = store();
    let a = AccountId::new();
    seed_file(dir.path(), &a, MAX_CONTACTS_PER_ACCOUNT as u32);
    let existing = many_nodes(1).next().unwrap();
    assert!(matches!(
        store.insert(&a, contact(1, "New")),
        Err(ContactStoreError::Full)
    ));
    assert!(store.get(&a, &existing).unwrap().is_some());
}

#[cfg(unix)]
#[test]
fn file_mode_is_0600_and_no_temp_file_remains() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    store.insert(&AccountId::new(), contact(1, "Bob")).unwrap();
    let mode = fs::metadata(file(&dir)).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [CONTACTS_FILE_NAME]);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[test]
fn file_contains_only_known_keys_and_no_key_material() {
    use crate::identity::testing::FakeKeychain;
    use crate::identity::{
        IdentityService, KeychainStore, MemorySettingsStore, NodeIdTicketProvider,
    };

    let ident_dir = tempfile::tempdir().unwrap();
    let keychain = std::sync::Arc::new(FakeKeychain::new());
    let service = IdentityService::new(
        std::sync::Arc::clone(&keychain),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        ident_dir.path(),
    );
    let provisioned = service.provision().unwrap();
    let secret = *keychain
        .load_identity_key(&provisioned.account_id)
        .unwrap()
        .as_bytes();

    let (dir, store) = store();
    let mut c = contact(1, "Bob");
    c.self_name = Some(name("Robert"));
    c.last_seen_ms = Some(9);
    store.insert(&provisioned.account_id, c).unwrap();
    let text = fs::read_to_string(file(&dir)).unwrap();
    assert!(!text.contains(&hex(&secret)));
    assert!(!text.contains(&base64(&secret)));

    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut top: Vec<_> = doc.as_object().unwrap().keys().cloned().collect();
    top.sort();
    assert_eq!(top, ["accounts", "version"]);
    let accounts = doc["accounts"].as_object().unwrap();
    assert_eq!(accounts.len(), 1);
    let contacts = accounts.values().next().unwrap().as_object().unwrap();
    let mut keys: Vec<_> = contacts
        .values()
        .next()
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "added_at_ms",
            "last_seen_ms",
            "nickname",
            "self_name",
            "trust"
        ]
    );
}
