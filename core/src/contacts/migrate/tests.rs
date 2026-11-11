use super::*;
use crate::contacts::contract::{contact, node};
use crate::contacts::testing::{FailingContactStore, MemoryContactStore};
use crate::contacts::FileContactStore;

type Peers<'a> = &'a [(u8, Option<&'a str>, i64)];

struct Legacy {
    dir: tempfile::TempDir,
    account: AccountId,
}

impl Legacy {
    fn new(peers: Peers) -> Self {
        Self::multi(&[(AccountId::new(), peers)])
    }

    fn multi(accounts: &[(AccountId, Peers)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = serde_json::Map::new();
        for (account, peers) in accounts {
            let map: serde_json::Map<_, _> = peers
                .iter()
                .map(|(seed, name, at)| {
                    (
                        node(*seed).to_string(),
                        serde_json::json!({"name": name, "added_at_ms": at}),
                    )
                })
                .collect();
            doc.insert(account.as_uuid().hyphenated().to_string(), map.into());
        }
        let body = serde_json::json!({"version": 1, "accounts": doc});
        fs::write(dir.path().join(LEGACY_FILE_NAME), body.to_string()).unwrap();
        Self {
            dir,
            account: accounts[0].0,
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.dir.path().join(LEGACY_FILE_NAME)
    }
}

#[test]
fn migrates_entries_as_unverified_contacts() {
    let l = Legacy::new(&[(1, Some("Bob"), 1234)]);
    let store = MemoryContactStore::new();
    let report = migrate_legacy_known_peers(l.dir.path(), &store).unwrap();
    assert_eq!(
        report,
        MigrationReport {
            migrated: 1,
            already_present: 0
        }
    );
    let c = store.get(&l.account, &node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Bob");
    assert_eq!(c.self_name.unwrap().as_str(), "Bob");
    assert_eq!(c.added_at_ms, 1234);
    assert_eq!(c.trust, TrustState::Unverified);
    assert_eq!(c.last_seen_ms, None);
}

#[test]
fn nameless_entry_gets_peer_short_id_nickname() {
    let l = Legacy::new(&[(1, None, 5)]);
    let store = MemoryContactStore::new();
    migrate_legacy_known_peers(l.dir.path(), &store).unwrap();
    let c = store.get(&l.account, &node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), format!("Peer {}", node(1).fmt_short()));
    assert_eq!(c.self_name, None);
}

#[test]
fn migrates_every_account_separately() {
    let (a, b) = (AccountId::new(), AccountId::new());
    let l = Legacy::multi(&[(a, &[(1, Some("Bob"), 1)]), (b, &[(2, None, 2)])]);
    let store = MemoryContactStore::new();
    let report = migrate_legacy_known_peers(l.dir.path(), &store).unwrap();
    assert_eq!(report.migrated, 2);
    assert!(store.get(&a, &node(1)).unwrap().is_some());
    assert!(store.get(&a, &node(2)).unwrap().is_none());
    assert!(store.get(&b, &node(2)).unwrap().is_some());
    assert!(store.get(&b, &node(1)).unwrap().is_none());
}

#[test]
fn is_idempotent_and_never_overwrites_contacts() {
    let l = Legacy::new(&[(1, Some("Bob"), 1), (2, None, 2)]);
    let store = MemoryContactStore::new();
    let mut mine = contact(1, "Mine");
    mine.trust = TrustState::Verified;
    store.insert(&l.account, mine.clone()).unwrap();
    let report = migrate_legacy_known_peers(l.dir.path(), &store).unwrap();
    assert_eq!(
        report,
        MigrationReport {
            migrated: 1,
            already_present: 1
        }
    );
    assert_eq!(store.get(&l.account, &node(1)).unwrap().unwrap(), mine);
    assert!(!l.path().exists());
    assert_eq!(
        migrate_legacy_known_peers(l.dir.path(), &store).unwrap(),
        MigrationReport::default()
    );
}

#[test]
fn single_atomic_write_and_legacy_deleted_only_after_success() {
    let l = Legacy::new(&[(1, Some("Bob"), 1), (2, None, 2), (3, None, 3)]);
    let store = MemoryContactStore::new();
    migrate_legacy_known_peers(l.dir.path(), &store).unwrap();
    assert_eq!(store.writes(), 1);
    assert!(!l.path().exists());

    let l = Legacy::new(&[(1, Some("Bob"), 1)]);
    assert!(matches!(
        migrate_legacy_known_peers(l.dir.path(), &FailingContactStore),
        Err(MigrationError::Store(_))
    ));
    assert!(l.path().exists());
}

#[test]
fn corrupt_legacy_file_is_left_untouched_and_nothing_migrated() {
    let good = node(1).to_string();
    let acct = AccountId::new().as_uuid().hyphenated().to_string();
    let cases = [
        "not json".to_owned(),
        r#"{"version":2,"accounts":{}}"#.to_owned(),
        r#"{"version":1,"accounts":{},"extra":1}"#.to_owned(),
        format!(
            r#"{{"version":1,"accounts":{{"x":{{"{good}":{{"name":null,"added_at_ms":1}}}}}}}}"#
        ),
        format!(
            r#"{{"version":1,"accounts":{{"{acct}":{{"nope":{{"name":null,"added_at_ms":1}}}}}}}}"#
        ),
        format!(
            r#"{{"version":1,"accounts":{{"{acct}":{{"{good}":{{"name":"a\nb","added_at_ms":1}}}}}}}}"#
        ),
    ];
    for body in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEGACY_FILE_NAME);
        fs::write(&path, &body).unwrap();
        let store = FileContactStore::new(dir.path());
        assert!(matches!(
            migrate_legacy_known_peers(dir.path(), &store),
            Err(MigrationError::LegacyCorrupt)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        assert!(!dir.path().join("contacts.json").exists());
    }
}

#[test]
fn missing_legacy_file_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileContactStore::new(dir.path());
    assert_eq!(
        migrate_legacy_known_peers(dir.path(), &store).unwrap(),
        MigrationReport::default()
    );
    assert!(!dir.path().join("contacts.json").exists());
}
