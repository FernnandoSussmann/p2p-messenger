use std::sync::Arc;

use super::*;
use crate::contacts::contract::{name, node};
use crate::contacts::testing::{FailingContactStore, MemoryContactStore, RecordingHistoryEraser};

const OWN: u8 = 100;

struct Fixture {
    book: ContactBook,
    store: Arc<MemoryContactStore>,
    eraser: Arc<RecordingHistoryEraser>,
}

fn fixture_with(eraser: RecordingHistoryEraser) -> Fixture {
    let store = Arc::new(MemoryContactStore::new());
    let eraser = Arc::new(eraser);
    let book = ContactBook::new(AccountId::new(), node(OWN), store.clone(), eraser.clone());
    Fixture {
        book,
        store,
        eraser,
    }
}

fn fixture() -> Fixture {
    fixture_with(RecordingHistoryEraser::new())
}

fn nick(book: &ContactBook, seed: u8) -> String {
    book.get(&node(seed))
        .unwrap()
        .unwrap()
        .nickname
        .as_str()
        .to_owned()
}

#[test]
fn add_makes_is_known_true_only_for_that_node() {
    let f = fixture();
    assert!(!f.book.is_known(&node(1)).unwrap());
    f.book.add(node(1), "Bob", None).unwrap();
    assert!(f.book.is_known(&node(1)).unwrap());
    assert!(!f.book.is_known(&node(2)).unwrap());
}

#[test]
fn add_starts_unverified() {
    let f = fixture();
    let c = f.book.add(node(1), "Bob", Some(name("Robert"))).unwrap();
    assert_eq!(c.trust, TrustState::Unverified);
    assert_eq!(c.self_name, Some(name("Robert")));
    assert_eq!(f.book.get(&node(1)).unwrap().unwrap(), c);
}

#[test]
fn add_rejects_own_node_id() {
    let f = fixture();
    assert!(matches!(
        f.book.add(node(OWN), "Me", None),
        Err(ContactError::SelfContact)
    ));
    assert!(f.book.list().unwrap().is_empty());
}

#[test]
fn add_invalid_nickname_persists_nothing() {
    let f = fixture();
    for bad in ["", "  ", "a\nb", "a\u{202E}b", "a\u{200B}b"] {
        assert!(matches!(
            f.book.add(node(1), bad, None),
            Err(ContactError::InvalidNickname(_))
        ));
    }
    assert!(f.book.list().unwrap().is_empty());
    assert_eq!(f.store.writes(), 0);
}

#[test]
fn add_existing_is_already_exists_and_keeps_record() {
    let f = fixture();
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    assert!(matches!(
        f.book.add(node(1), "Other", None),
        Err(ContactError::AlreadyExists)
    ));
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Bob");
    assert_eq!(c.trust, TrustState::Verified);
}

#[test]
fn remember_dialed_creates_unverified_with_default_nickname() {
    let f = fixture();
    f.book
        .remember_dialed(node(1), Some(&name("Alice")))
        .unwrap();
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Alice");
    assert_eq!(c.self_name, Some(name("Alice")));
    assert_eq!(c.trust, TrustState::Unverified);

    f.book.remember_dialed(node(2), None).unwrap();
    let c = f.book.get(&node(2)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), format!("Peer {}", node(2).fmt_short()));
    assert_eq!(c.self_name, None);
}

#[test]
fn remember_dialed_keeps_existing_nickname_and_trust() {
    let f = fixture();
    f.book.add(node(1), "Mine", None).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    f.book
        .remember_dialed(node(1), Some(&name("Alice")))
        .unwrap();
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Mine");
    assert_eq!(c.trust, TrustState::Verified);
    assert_eq!(c.self_name, Some(name("Alice")));
}

#[test]
fn rename_keeps_node_id_self_name_and_trust() {
    let f = fixture();
    f.book.add(node(1), "Bob", Some(name("Robert"))).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    let c = f.book.rename(&node(1), "  Bobby ").unwrap();
    assert_eq!(c.node_id, node(1));
    assert_eq!(c.nickname.as_str(), "Bobby");
    assert_eq!(c.self_name, Some(name("Robert")));
    assert_eq!(c.trust, TrustState::Verified);
    assert!(matches!(
        f.book.rename(&node(1), "a\nb"),
        Err(ContactError::InvalidNickname(_))
    ));
    assert_eq!(nick(&f.book, 1), "Bobby");
}

#[test]
fn observe_hello_keeps_nickname_and_trust() {
    let f = fixture();
    f.book.add(node(1), "Bobby", Some(name("Bob"))).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    f.book
        .observe_hello(&node(1), Some(&name("Robert")))
        .unwrap();
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Bobby");
    assert_eq!(c.trust, TrustState::Verified);
    assert_eq!(c.self_name, Some(name("Robert")));
    assert!(c.last_seen_ms.is_some());
    f.book.observe_hello(&node(1), None).unwrap();
    assert_eq!(f.book.get(&node(1)).unwrap().unwrap().self_name, None);
}

#[test]
fn default_nickname_is_a_snapshot() {
    let f = fixture();
    f.book.remember_dialed(node(1), Some(&name("Bob"))).unwrap();
    f.book
        .observe_hello(&node(1), Some(&name("Robert")))
        .unwrap();
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.nickname.as_str(), "Bob");
    assert_eq!(c.self_name, Some(name("Robert")));
}

#[test]
fn observe_lan_updates_self_name_only() {
    let f = fixture();
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.observe_lan(&node(1), &name("Robert")).unwrap();
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.self_name, Some(name("Robert")));
    assert_eq!(c.last_seen_ms, None);
    assert_eq!(c.nickname.as_str(), "Bob");
    f.book.observe_lan(&node(2), &name("Ghost")).unwrap();
    assert!(f.book.get(&node(2)).unwrap().is_none());
}

#[test]
fn only_set_verified_changes_trust() {
    let f = fixture();
    let trust = |f: &Fixture| f.book.get(&node(1)).unwrap().unwrap().trust;
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.rename(&node(1), "Bobby").unwrap();
    f.book
        .observe_hello(&node(1), Some(&name("Verified")))
        .unwrap();
    f.book.observe_lan(&node(1), &name("✓ Bob")).unwrap();
    f.book
        .remember_dialed(node(1), Some(&name("✓ Bob")))
        .unwrap();
    assert!(f.book.add(node(1), "Again", None).is_err());
    assert_eq!(trust(&f), TrustState::Unverified);

    f.book.set_verified(&node(1), true).unwrap();
    assert_eq!(trust(&f), TrustState::Verified);
    f.book
        .observe_hello(&node(1), Some(&name("Other")))
        .unwrap();
    f.book.observe_lan(&node(1), &name("Other 2")).unwrap();
    f.book.remember_dialed(node(1), None).unwrap();
    f.book.rename(&node(1), "Renamed").unwrap();
    assert_eq!(trust(&f), TrustState::Verified);

    f.book.set_verified(&node(1), false).unwrap();
    assert_eq!(trust(&f), TrustState::Unverified);
}

#[test]
fn observe_unknown_never_creates_a_contact() {
    let f = fixture();
    f.book.observe_hello(&node(1), Some(&name("Eve"))).unwrap();
    f.book.observe_lan(&node(1), &name("Eve")).unwrap();
    assert!(!f.book.is_known(&node(1)).unwrap());
    assert!(f.book.list().unwrap().is_empty());
}

#[test]
fn set_verified_and_rename_unknown_are_not_found() {
    let f = fixture();
    assert!(matches!(
        f.book.set_verified(&node(1), true),
        Err(ContactError::NotFound)
    ));
    assert!(matches!(
        f.book.rename(&node(1), "Bob"),
        Err(ContactError::NotFound)
    ));
}

#[test]
fn remove_reverts_trust() {
    let f = fixture();
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    let removal = f.book.remove(&node(1), HistoryChoice::Keep).unwrap();
    assert!(!removal.history_erased);
    assert!(!f.book.is_known(&node(1)).unwrap());
    f.book.add(node(1), "Bob", None).unwrap();
    assert_eq!(
        f.book.get(&node(1)).unwrap().unwrap().trust,
        TrustState::Unverified
    );
}

#[test]
fn remove_keep_never_calls_eraser() {
    let f = fixture();
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.remove(&node(1), HistoryChoice::Keep).unwrap();
    assert!(f.eraser.calls().is_empty());
}

#[test]
fn remove_erase_calls_eraser_once_then_removes() {
    let f = fixture();
    f.book.add(node(1), "Bob", None).unwrap();
    let removal = f.book.remove(&node(1), HistoryChoice::Erase).unwrap();
    assert!(removal.history_erased);
    assert_eq!(f.eraser.calls(), vec![(*f.book.account(), node(1))]);
    assert!(!f.book.is_known(&node(1)).unwrap());
}

#[test]
fn remove_keeps_contact_when_history_erase_fails() {
    let f = fixture_with(RecordingHistoryEraser::failing());
    f.book.add(node(1), "Bob", None).unwrap();
    f.book.set_verified(&node(1), true).unwrap();
    assert!(matches!(
        f.book.remove(&node(1), HistoryChoice::Erase),
        Err(ContactError::HistoryEraseFailed)
    ));
    assert_eq!(f.eraser.calls().len(), 1);
    let c = f.book.get(&node(1)).unwrap().unwrap();
    assert_eq!(c.trust, TrustState::Verified);
}

#[test]
fn remove_unknown_is_not_found_and_erases_nothing() {
    let f = fixture();
    assert!(matches!(
        f.book.remove(&node(1), HistoryChoice::Erase),
        Err(ContactError::NotFound)
    ));
    assert!(f.eraser.calls().is_empty());
}

#[test]
fn books_for_different_accounts_are_isolated() {
    let store = Arc::new(MemoryContactStore::new());
    let eraser = Arc::new(RecordingHistoryEraser::new());
    let mk = || ContactBook::new(AccountId::new(), node(OWN), store.clone(), eraser.clone());
    let (a, b) = (mk(), mk());
    a.add(node(1), "Bob", None).unwrap();
    assert!(a.is_known(&node(1)).unwrap());
    assert!(!b.is_known(&node(1)).unwrap());
    b.add(node(1), "Other Bob", None).unwrap();
    a.remove(&node(1), HistoryChoice::Keep).unwrap();
    assert!(b.is_known(&node(1)).unwrap());
}

#[test]
fn store_failure_is_unavailable_never_unknown() {
    let book = ContactBook::new(
        AccountId::new(),
        node(OWN),
        Arc::new(FailingContactStore),
        Arc::new(RecordingHistoryEraser::new()),
    );
    assert!(matches!(
        book.is_known(&node(1)),
        Err(ContactError::Unavailable)
    ));
    assert!(matches!(book.list(), Err(ContactError::Unavailable)));
}

#[test]
fn list_is_sorted_by_nickname_then_node_id() {
    let f = fixture();
    f.book.add(node(1), "bob", None).unwrap();
    f.book.add(node(2), "Alice", None).unwrap();
    f.book.add(node(3), "Bob", None).unwrap();
    let list = f.book.list().unwrap();
    let nicks: Vec<_> = list.iter().map(|c| c.nickname.as_str()).collect();
    assert_eq!(nicks[0], "Alice");
    assert_eq!(
        nicks[1..]
            .iter()
            .map(|n| n.to_lowercase())
            .collect::<Vec<_>>(),
        ["bob", "bob"]
    );
    assert!(list[1].node_id.to_string() < list[2].node_id.to_string());
}
