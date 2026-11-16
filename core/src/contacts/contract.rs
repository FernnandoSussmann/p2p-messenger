//! The behaviour every [`ContactStore`] must have, run against each impl.

use super::model::{Contact, NameUpdate, Nickname, TrustState};
use super::store::{ContactStore, ContactStoreError, LAST_SEEN_RESOLUTION_MS};
use crate::identity::{validate_display_name, AccountId, DisplayName, NodeId};

pub(crate) fn node(seed: u8) -> NodeId {
    iroh_base::SecretKey::from_bytes(&[seed; 32]).public()
}

pub(crate) fn name(raw: &str) -> DisplayName {
    validate_display_name(raw).unwrap()
}

pub(crate) fn contact(seed: u8, nickname: &str) -> Contact {
    Contact {
        node_id: node(seed),
        nickname: Nickname::parse(nickname).unwrap(),
        self_name: None,
        trust: TrustState::Unverified,
        added_at_ms: 1_000,
        last_seen_ms: None,
    }
}

/// Everything a store must do for one account. `a` must be an account the
/// store can serve (the SQL store opens one account per `Storage`).
pub(crate) fn contract_suite(store: &dyn ContactStore, a: AccountId) {
    assert!(store.get(&a, &node(1)).unwrap().is_none());
    assert!(store.list(&a).unwrap().is_empty());
    store.insert(&a, contact(1, "Bob")).unwrap();
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), contact(1, "Bob"));
    assert_eq!(store.list(&a).unwrap().len(), 1);

    // Insert never overwrites.
    let mut other = contact(1, "Other");
    other.trust = TrustState::Verified;
    assert!(matches!(
        store.insert(&a, other),
        Err(ContactStoreError::AlreadyExists)
    ));
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), contact(1, "Bob"));

    // set_nickname changes only the nickname.
    let renamed = store
        .set_nickname(&a, &node(1), &Nickname::parse("Bobby").unwrap())
        .unwrap();
    let mut expected = contact(1, "Bobby");
    assert_eq!(renamed, expected);
    assert!(matches!(
        store.set_nickname(&a, &node(9), &Nickname::parse("x").unwrap()),
        Err(ContactStoreError::NotFound)
    ));

    // set_trust changes only trust.
    let verified = store.set_trust(&a, &node(1), TrustState::Verified).unwrap();
    expected.trust = TrustState::Verified;
    assert_eq!(verified, expected);
    assert!(matches!(
        store.set_trust(&a, &node(9), TrustState::Verified),
        Err(ContactStoreError::NotFound)
    ));

    // observe changes only self_name / last_seen.
    let robert = name("Robert");
    store
        .observe(&a, &node(1), NameUpdate::Set(Some(&robert)), Some(5_000))
        .unwrap();
    expected.self_name = Some(robert.clone());
    expected.last_seen_ms = Some(5_000);
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), expected);
    store.observe(&a, &node(1), NameUpdate::Keep, None).unwrap();
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), expected);
    store
        .observe(
            &a,
            &node(1),
            NameUpdate::Keep,
            Some(5_000 + LAST_SEEN_RESOLUTION_MS),
        )
        .unwrap();
    expected.last_seen_ms = Some(5_000 + LAST_SEEN_RESOLUTION_MS);
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), expected);
    store
        .observe(&a, &node(1), NameUpdate::Set(None), None)
        .unwrap();
    expected.self_name = None;
    assert_eq!(store.get(&a, &node(1)).unwrap().unwrap(), expected);

    // observe on an unknown id creates nothing.
    store
        .observe(&a, &node(7), NameUpdate::Set(Some(&robert)), Some(1))
        .unwrap();
    assert!(store.get(&a, &node(7)).unwrap().is_none());
    assert_eq!(store.list(&a).unwrap().len(), 1);

    // insert_missing skips existing entries untouched.
    let report = store
        .insert_missing(vec![
            (a, contact(1, "Clobber")),
            (a, contact(2, "Two")),
            (a, contact(3, "Three")),
            (a, contact(3, "Three again")),
        ])
        .unwrap();
    assert_eq!((report.inserted, report.already_present), (2, 2));
    assert_eq!(
        store.get(&a, &node(1)).unwrap().unwrap().nickname.as_str(),
        "Bobby"
    );
    assert_eq!(
        store.get(&a, &node(3)).unwrap().unwrap().nickname.as_str(),
        "Three"
    );
    assert!(store.remove(&a, &node(3)).unwrap());

    // remove reports whether anything was removed.
    assert!(store.remove(&a, &node(2)).unwrap());
    assert!(!store.remove(&a, &node(2)).unwrap());
    assert!(store.get(&a, &node(2)).unwrap().is_none());
}

/// Two accounts never see each other. `store_a` serves `a` and `store_b`
/// serves `b`; for a store that serves every account they are the same.
pub(crate) fn isolation_suite(
    store_a: &dyn ContactStore,
    a: AccountId,
    store_b: &dyn ContactStore,
    b: AccountId,
) {
    store_a.insert(&a, contact(1, "Mine")).unwrap();
    assert!(store_b.get(&b, &node(1)).unwrap().is_none());
    assert!(store_b.list(&b).unwrap().is_empty());
    store_b.insert(&b, contact(1, "Elsewhere")).unwrap();
    store_b
        .set_trust(&b, &node(1), TrustState::Verified)
        .unwrap();
    let robert = name("Robert");
    store_b
        .observe(&b, &node(1), NameUpdate::Set(Some(&robert)), Some(9))
        .unwrap();
    assert_eq!(
        store_a.get(&a, &node(1)).unwrap().unwrap(),
        contact(1, "Mine")
    );
    assert!(store_b.remove(&b, &node(1)).unwrap());
    assert!(store_a.get(&a, &node(1)).unwrap().is_some());
    store_b
        .insert_missing(vec![(b, contact(3, "Three"))])
        .unwrap();
    assert!(store_a.get(&a, &node(3)).unwrap().is_none());
}

/// Concurrent read-modify-write must neither deadlock nor lose the contact.
pub(crate) fn concurrent_observers(store: &dyn ContactStore, a: AccountId) {
    store.insert(&a, contact(5, "Racer")).unwrap();
    std::thread::scope(|scope| {
        for i in 0..8u8 {
            scope.spawn(move || {
                let n = name(&format!("Name{i}"));
                for round in 0..10i64 {
                    store
                        .observe(
                            &a,
                            &node(5),
                            NameUpdate::Set(Some(&n)),
                            Some(round * LAST_SEEN_RESOLUTION_MS),
                        )
                        .unwrap();
                }
            });
        }
    });
    let c = store.get(&a, &node(5)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Racer");
    assert!(c.self_name.unwrap().as_str().starts_with("Name"));
}
