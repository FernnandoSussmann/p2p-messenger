use std::time::Duration;

use messenger_core::contacts::testing::{
    FailingContactStore, MemoryContactStore, RecordingHistoryEraser,
};
use messenger_core::discovery::testing::{MemoryLanBus, TestOptions};
use messenger_core::discovery::{ConnectionState, Direction};
use messenger_core::identity::KeychainStore;

use super::*;
use crate::net_cmd::{connect_ticket, lan_peers, pending_incoming};
use crate::test_util::{
    assert_forms_absent, events, expect, is_incoming, is_state, started, started_with, ticket_of,
    Started, WAIT,
};

const DTO_KEYS: [&str; 6] = [
    "added_at_ms",
    "last_seen_ms",
    "nickname",
    "node_id",
    "self_name",
    "verified",
];

fn node(seed: u8) -> NodeId {
    iroh::SecretKey::from_bytes(&[seed; 32]).public()
}

#[test]
fn contacts_state_bind_is_idempotent() {
    let state = ContactsState::new(
        Arc::new(MemoryContactStore::new()),
        Arc::new(RecordingHistoryEraser::new()),
    );
    assert!(state.book().is_none());
    let first = state.bind(AccountId::new(), node(1));
    let second = state.bind(AccountId::new(), node(2));
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.account(), second.account());
    assert!(Arc::ptr_eq(&state.book().unwrap(), &first));
}

fn bound(store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser>) -> ContactsState {
    let c = ContactsState::new(store, history);
    c.bind(AccountId::new(), node(1));
    c
}

fn memory() -> ContactsState {
    bound(
        Arc::new(MemoryContactStore::new()),
        Arc::new(RecordingHistoryEraser::new()),
    )
}

fn id(seed: u8) -> String {
    node(seed).to_string()
}

fn unbound() -> ContactsState {
    ContactsState::new(
        Arc::new(MemoryContactStore::new()),
        Arc::new(RecordingHistoryEraser::new()),
    )
}

#[tokio::test]
async fn contacts_commands_not_ready_before_bind() {
    let (c, net, peer) = (unbound(), NetState::default(), id(2));
    let not_ready = ContactsCmdError::NotReady;
    assert_eq!(list(&c).await.unwrap_err(), not_ready);
    assert_eq!(add(&c, &net, &peer, "x").await.unwrap_err(), not_ready);
    assert_eq!(rename(&c, &peer, "x").await.unwrap_err(), not_ready);
    assert_eq!(set_verified(&c, &peer, true).await.unwrap_err(), not_ready);
    assert_eq!(remove(&c, &net, &peer, None).await.unwrap_err(), not_ready);
    assert_eq!(is_known(&c, &peer).await.unwrap_err(), not_ready);
}

#[test]
fn contacts_errors_serialize_with_type_tag() {
    for (err, tag) in [
        (ContactsCmdError::NotReady, "NotReady"),
        (ContactsCmdError::InvalidNodeId, "InvalidNodeId"),
        (ContactsCmdError::SelfContact, "SelfContact"),
        (ContactsCmdError::AlreadyExists, "AlreadyExists"),
        (ContactsCmdError::NotFound, "NotFound"),
        (ContactsCmdError::Full, "Full"),
        (ContactsCmdError::StoreUnavailable, "StoreUnavailable"),
        (ContactsCmdError::HistoryEraseFailed, "HistoryEraseFailed"),
    ] {
        assert_eq!(
            serde_json::to_value(err).unwrap(),
            serde_json::json!({ "type": tag })
        );
    }
    assert_eq!(
        serde_json::to_value(ContactsCmdError::InvalidNickname {
            kind: DisplayNameKind::InvisibleCharacter
        })
        .unwrap(),
        serde_json::json!({ "type": "InvalidNickname", "kind": "InvisibleCharacter" })
    );
}

#[tokio::test]
async fn contact_dto_json_shape() {
    let (c, net) = (memory(), NetState::default());
    let dto = add(&c, &net, &id(2), "Bobby").await.unwrap();
    let mut keys: Vec<String> = serde_json::to_value(&dto)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, DTO_KEYS);
    let removed = remove(&c, &net, &id(2), None).await.unwrap();
    let mut keys: Vec<String> = serde_json::to_value(&removed)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, ["history_erased", "node_id"]);
}

#[tokio::test]
async fn contacts_add_then_list_shows_unverified() {
    let (c, net) = (memory(), NetState::default());
    let dto = add(&c, &net, &id(2), "  Bobby  ").await.unwrap();
    assert!(!dto.verified);
    assert_eq!(dto.nickname, "Bobby");
    let listed = list(&c).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].node_id, id(2));
    assert!(!listed[0].verified);
}

#[tokio::test(flavor = "multi_thread")]
async fn contacts_add_takes_self_name_from_the_node_not_args() {
    let bus = MemoryLanBus::new();
    let a = started_with("Alice", TestOptions::default().with_lan(&bus)).await;
    let b = started_with("Bob", TestOptions::default().with_lan(&bus)).await;
    let deadline = tokio::time::Instant::now() + WAIT;
    while !lan_peers(&a.state)
        .unwrap()
        .iter()
        .any(|p| p.node_id == b.id.to_string())
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "B never seen on LAN"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let dto = add(&a.contacts, &a.state, &b.id.to_string(), "Bobby")
        .await
        .unwrap();
    assert_eq!(dto.nickname, "Bobby");
    assert_eq!(dto.self_name.as_deref(), Some("Bob"));
}

#[tokio::test]
async fn contacts_add_without_running_node_has_no_self_name() {
    let (c, net) = (memory(), NetState::default());
    let dto = add(&c, &net, &id(2), "Bobby").await.unwrap();
    assert_eq!(dto.self_name, None);
}

#[tokio::test]
async fn contacts_add_rejects_malformed_node_id() {
    let (c, net) = (memory(), NetState::default());
    let valid_with_space = format!("{} ", id(2));
    for bad in [
        "",
        "x",
        "\u{202E}",
        &"f".repeat(4096),
        valid_with_space.as_str(),
    ] {
        assert_eq!(
            add(&c, &net, bad, "Bobby").await.unwrap_err(),
            ContactsCmdError::InvalidNodeId,
            "{bad:?}"
        );
    }
    assert!(list(&c).await.unwrap().is_empty());
}

#[tokio::test]
async fn contacts_add_rejects_self() {
    let (c, net) = (memory(), NetState::default());
    assert_eq!(
        add(&c, &net, &id(1), "Me").await.unwrap_err(),
        ContactsCmdError::SelfContact
    );
}

#[tokio::test]
async fn contacts_add_invalid_nickname_kinds() {
    let (c, net) = (memory(), NetState::default());
    let too_long = "a".repeat(500);
    for (nick, kind) in [
        ("", DisplayNameKind::Empty),
        ("a\u{0007}b", DisplayNameKind::ControlCharacter),
        ("a\u{2067}b", DisplayNameKind::BidiOverride),
        ("a\u{200B}b", DisplayNameKind::InvisibleCharacter),
        ("a\u{FEFF}b", DisplayNameKind::InvisibleCharacter),
        (too_long.as_str(), DisplayNameKind::TooLong),
    ] {
        assert_eq!(
            add(&c, &net, &id(2), nick).await.unwrap_err(),
            ContactsCmdError::InvalidNickname { kind },
            "{nick:?}"
        );
    }
    assert!(list(&c).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn contacts_add_existing_is_already_exists() {
    let (c, net) = (memory(), NetState::default());
    add(&c, &net, &id(2), "Bobby").await.unwrap();
    assert_eq!(
        add(&c, &net, &id(2), "Other").await.unwrap_err(),
        ContactsCmdError::AlreadyExists
    );

    let a = started("Alice").await;
    let b = started("Bob").await;
    let mut eb = events(&b);
    connect_ticket(&a.state, &ticket_of(&b).await).unwrap();
    expect(&mut eb, is_incoming).await;
    respond(&b, &a.id).await;
    expect(
        &mut events(&a),
        is_state(b.id, Direction::Outbound, ConnectionState::Connected),
    )
    .await;
    assert_eq!(
        add(&a.contacts, &a.state, &b.id.to_string(), "Again")
            .await
            .unwrap_err(),
        ContactsCmdError::AlreadyExists
    );
}

async fn respond(s: &Started, peer: &NodeId) {
    crate::net_cmd::respond_incoming(
        &s.state,
        &s.contacts,
        &peer.to_string(),
        true,
        Some("Peer".into()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn contacts_rename_validates_and_keeps_node_id_and_self_name() {
    let (c, net) = (memory(), NetState::default());
    let before = add(&c, &net, &id(2), "Bobby").await.unwrap();
    let renamed = rename(&c, &id(2), " Robert ").await.unwrap();
    assert_eq!(renamed.nickname, "Robert");
    assert_eq!(renamed.node_id, before.node_id);
    assert_eq!(renamed.self_name, before.self_name);
    assert_eq!(
        rename(&c, &id(2), "a\u{200B}b").await.unwrap_err(),
        ContactsCmdError::InvalidNickname {
            kind: DisplayNameKind::InvisibleCharacter
        }
    );
    assert_eq!(list(&c).await.unwrap()[0].nickname, "Robert");
    assert_eq!(
        rename(&c, &id(3), "Nobody").await.unwrap_err(),
        ContactsCmdError::NotFound
    );
}

#[tokio::test]
async fn contacts_set_verified_round_trip_and_not_found() {
    let (c, net) = (memory(), NetState::default());
    add(&c, &net, &id(2), "Bobby").await.unwrap();
    assert!(set_verified(&c, &id(2), true).await.unwrap().verified);
    assert!(list(&c).await.unwrap()[0].verified);
    assert!(!set_verified(&c, &id(2), false).await.unwrap().verified);
    assert_eq!(
        set_verified(&c, &id(3), true).await.unwrap_err(),
        ContactsCmdError::NotFound
    );
}

#[tokio::test]
async fn contacts_remove_defaults_to_keeping_history() {
    for flag in [None, Some(false)] {
        let history = Arc::new(RecordingHistoryEraser::new());
        let c = bound(Arc::new(MemoryContactStore::new()), history.clone());
        let net = NetState::default();
        add(&c, &net, &id(2), "Bobby").await.unwrap();
        let r = remove(&c, &net, &id(2), flag).await.unwrap();
        assert!(!r.history_erased);
        assert_eq!(r.node_id, id(2));
        assert!(history.calls().is_empty());
        assert!(list(&c).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn contacts_remove_with_clear_history_erases_then_removes() {
    let history = Arc::new(RecordingHistoryEraser::new());
    let c = bound(Arc::new(MemoryContactStore::new()), history.clone());
    let net = NetState::default();
    add(&c, &net, &id(2), "Bobby").await.unwrap();
    let r = remove(&c, &net, &id(2), Some(true)).await.unwrap();
    assert!(r.history_erased);
    assert_eq!(history.calls().len(), 1);
    assert_eq!(history.calls()[0].1, node(2));
    assert!(list(&c).await.unwrap().is_empty());
}

#[tokio::test]
async fn contacts_remove_history_failure_keeps_contact() {
    let history = Arc::new(RecordingHistoryEraser::failing());
    let c = bound(Arc::new(MemoryContactStore::new()), history.clone());
    let net = NetState::default();
    add(&c, &net, &id(2), "Bobby").await.unwrap();
    assert_eq!(
        remove(&c, &net, &id(2), Some(true)).await.unwrap_err(),
        ContactsCmdError::HistoryEraseFailed
    );
    assert_eq!(list(&c).await.unwrap().len(), 1);
}

#[tokio::test]
async fn contacts_remove_unknown_is_not_found() {
    let history = Arc::new(RecordingHistoryEraser::new());
    let c = bound(Arc::new(MemoryContactStore::new()), history.clone());
    assert_eq!(
        remove(&c, &NetState::default(), &id(2), Some(true))
            .await
            .unwrap_err(),
        ContactsCmdError::NotFound
    );
    assert!(history.calls().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn contacts_remove_drops_live_connection_and_next_inbound_prompts() {
    let a = started("Alice").await;
    let b = started("Bob").await;
    let (mut ea, mut eb) = (events(&a), events(&b));
    add(&a.contacts, &a.state, &b.id.to_string(), "Bobby")
        .await
        .unwrap();
    connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
    expect(
        &mut eb,
        is_state(a.id, Direction::Outbound, ConnectionState::Connected),
    )
    .await;

    remove(&a.contacts, &a.state, &b.id.to_string(), None)
        .await
        .unwrap();
    expect(
        &mut eb,
        is_state(a.id, Direction::Outbound, ConnectionState::Disconnected),
    )
    .await;

    connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
    expect(&mut ea, is_incoming).await;
    assert_eq!(pending_incoming(&a.state).unwrap().len(), 1);
    assert!(!is_known(&a.contacts, &b.id.to_string()).await.unwrap());
}

#[tokio::test]
async fn contacts_is_known_tracks_add_and_remove() {
    let (c, net) = (memory(), NetState::default());
    assert!(!is_known(&c, &id(2)).await.unwrap());
    add(&c, &net, &id(2), "Bobby").await.unwrap();
    assert!(is_known(&c, &id(2)).await.unwrap());
    remove(&c, &net, &id(2), None).await.unwrap();
    assert!(!is_known(&c, &id(2)).await.unwrap());
    assert_eq!(
        is_known(&c, "nope").await.unwrap_err(),
        ContactsCmdError::InvalidNodeId
    );
}

#[tokio::test]
async fn contacts_is_known_store_failure_is_error_not_false() {
    let c = bound(
        Arc::new(FailingContactStore),
        Arc::new(RecordingHistoryEraser::new()),
    );
    assert_eq!(
        is_known(&c, &id(2)).await.unwrap_err(),
        ContactsCmdError::StoreUnavailable
    );
    assert_eq!(
        list(&c).await.unwrap_err(),
        ContactsCmdError::StoreUnavailable
    );
}

#[tokio::test]
async fn contacts_are_namespaced_by_bound_account() {
    let store: Arc<dyn ContactStore> = Arc::new(MemoryContactStore::new());
    let history: Arc<dyn HistoryEraser> = Arc::new(RecordingHistoryEraser::new());
    let one = bound(store.clone(), history.clone());
    let two = bound(store, history);
    let net = NetState::default();
    add(&one, &net, &id(2), "Bobby").await.unwrap();
    assert!(list(&two).await.unwrap().is_empty());
    assert!(!is_known(&two, &id(2)).await.unwrap());
    assert_eq!(
        remove(&two, &net, &id(2), None).await.unwrap_err(),
        ContactsCmdError::NotFound
    );
    assert!(is_known(&one, &id(2)).await.unwrap());
}

#[tokio::test]
async fn contact_dtos_never_contain_key_material() {
    let a = started("Alice").await;
    let account = a.identity.load().unwrap().account_id;
    let key = *a.keychain.load_identity_key(&account).unwrap().as_bytes();
    add(&a.contacts, &a.state, &id(2), "Bobby").await.unwrap();
    let json = serde_json::to_string(&list(&a.contacts).await.unwrap()).unwrap();
    assert_forms_absent(&json, &key);
}
