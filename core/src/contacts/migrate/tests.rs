use super::*;
use crate::contacts::contract::node;

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

    fn read(&self) -> Vec<(AccountId, Contact)> {
        read_legacy_known_peers(self.dir.path()).unwrap().unwrap()
    }
}

#[test]
fn entries_become_unverified_contacts() {
    let l = Legacy::new(&[(1, Some("Bob"), 1234)]);
    let parsed = l.read();
    assert_eq!(parsed.len(), 1);
    let (account, c) = &parsed[0];
    assert_eq!(*account, l.account);
    assert_eq!(c.node_id, node(1));
    assert_eq!(c.nickname.as_str(), "Bob");
    assert_eq!(c.self_name.as_ref().unwrap().as_str(), "Bob");
    assert_eq!(c.added_at_ms, 1234);
    assert_eq!(c.trust, TrustState::Unverified);
    assert_eq!(c.last_seen_ms, None);
}

#[test]
fn nameless_entry_gets_peer_short_id_nickname() {
    let c = Legacy::new(&[(1, None, 5)]).read().remove(0).1;
    assert_eq!(c.nickname.as_str(), format!("Peer {}", node(1).fmt_short()));
    assert_eq!(c.self_name, None);
}

#[test]
fn invisible_legacy_name_counts_as_no_name() {
    let c = Legacy::new(&[(1, Some("Bob\u{200D}"), 5)])
        .read()
        .remove(0)
        .1;
    assert_eq!(c.nickname.as_str(), format!("Peer {}", node(1).fmt_short()));
    assert_eq!(c.self_name, None);
}

#[test]
fn every_account_is_read_separately() {
    let (a, b) = (AccountId::new(), AccountId::new());
    let l = Legacy::multi(&[(a, &[(1, Some("Bob"), 1)]), (b, &[(2, None, 2)])]);
    let parsed = l.read();
    assert_eq!(parsed.len(), 2);
    assert!(parsed
        .iter()
        .any(|(acc, c)| *acc == a && c.node_id == node(1)));
    assert!(parsed
        .iter()
        .any(|(acc, c)| *acc == b && c.node_id == node(2)));
}

#[test]
fn corrupt_legacy_file_is_a_typed_error_and_untouched() {
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
        format!(
            r#"{{"version":1,"accounts":{{"{acct}":{{"{good}":{{"name":null,"added_at_ms":-1}}}}}}}}"#
        ),
    ];
    for body in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEGACY_FILE_NAME);
        fs::write(&path, &body).unwrap();
        assert_eq!(
            read_legacy_known_peers(dir.path()),
            Err(LegacyError::Corrupt)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
    }
}

#[test]
fn oversized_file_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(LEGACY_FILE_NAME),
        vec![b' '; MAX_LEGACY_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(
        read_legacy_known_peers(dir.path()),
        Err(LegacyError::Corrupt)
    );
}

#[cfg(unix)]
#[test]
fn symlinked_file_is_not_regular() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere");
    fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(LEGACY_FILE_NAME)).unwrap();
    assert_eq!(
        read_legacy_known_peers(dir.path()),
        Err(LegacyError::NotRegularFile)
    );
}

#[test]
fn missing_legacy_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_legacy_known_peers(dir.path()), Ok(None));
}
