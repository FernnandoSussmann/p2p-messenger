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

pub(crate) fn contract_suite(store: &dyn ContactStore) {
    let a = AccountId::new();
    let b = AccountId::new();

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

    // Accounts are isolated.
    assert!(store.get(&b, &node(1)).unwrap().is_none());
    store.insert(&b, contact(1, "Elsewhere")).unwrap();
    assert_eq!(
        store.get(&a, &node(1)).unwrap().unwrap().nickname.as_str(),
        "Bobby"
    );
    assert!(store.remove(&b, &node(1)).unwrap());
    assert!(store.get(&a, &node(1)).unwrap().is_some());

    // insert_missing skips existing entries untouched.
    let report = store
        .insert_missing(vec![
            (a, contact(1, "Clobber")),
            (a, contact(2, "Two")),
            (b, contact(3, "Three")),
            (b, contact(3, "Three again")),
        ])
        .unwrap();
    assert_eq!((report.inserted, report.already_present), (2, 2));
    assert_eq!(
        store.get(&a, &node(1)).unwrap().unwrap().nickname.as_str(),
        "Bobby"
    );
    assert_eq!(
        store.get(&b, &node(3)).unwrap().unwrap().nickname.as_str(),
        "Three"
    );

    // remove reports whether anything was removed.
    assert!(store.remove(&a, &node(2)).unwrap());
    assert!(!store.remove(&a, &node(2)).unwrap());
    assert!(store.get(&a, &node(2)).unwrap().is_none());
}
