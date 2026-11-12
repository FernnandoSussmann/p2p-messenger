//! The behaviour every [`MessageStore`] must have, run against each impl.

use protocol::{new_message_id, ChatMessage};

use super::model::{
    HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError, StoredMessage,
};
use super::store::{history_page, MessageStore, MAX_PAGE, MAX_SEARCH_RESULTS};
use crate::identity::{AccountId, NodeId};

pub(crate) fn node() -> NodeId {
    iroh_base::SecretKey::generate().public()
}

pub(crate) fn chat(sent_at_ms: i64, body: &str) -> ChatMessage {
    ChatMessage {
        id: new_message_id(),
        sent_at_ms,
        body: body.to_owned(),
    }
}

fn out(store: &dyn MessageStore, a: &AccountId, p: &NodeId, at: i64, body: &str) -> StoredMessage {
    store
        .insert_outgoing(a, p, &chat(at, body), MessageStatus::Sending)
        .unwrap()
}

fn incoming(
    store: &dyn MessageStore,
    a: &AccountId,
    p: &NodeId,
    at: i64,
    body: &str,
) -> StoredMessage {
    match store.insert_incoming(a, p, &chat(at, body), 5).unwrap() {
        Insert::Inserted(row) => row,
        Insert::Duplicate => panic!("unexpected duplicate"),
    }
}

fn ids(rows: &[StoredMessage]) -> Vec<protocol::MessageId> {
    rows.iter().map(|m| m.id).collect()
}

pub(crate) fn contract_suite(store: &dyn MessageStore) {
    insert_round_trips_every_field(store);
    duplicates_are_detected(store);
    status_lifecycle(store);
    page_orders_by_sent_at_then_seq(store);
    keyset_paging_has_no_gaps_or_duplicates(store);
    undelivered_is_bounded_and_ordered(store);
    limits_are_clamped(store);
    search_behaviour(store);
    search_treats_sql_and_fts_syntax_as_text(store);
    accounts_are_isolated(store);
    erase_removes_only_that_peer(store);
}

fn insert_round_trips_every_field(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let body = "line one\nline two 👋🏽 ünï";
    for at in [i64::MIN, -5, 0, i64::MAX] {
        let sent = store
            .insert_outgoing(&a, &p, &chat(at, body), MessageStatus::Pending)
            .unwrap();
        assert_eq!(sent.direction, MessageDirection::Outgoing);
        assert_eq!(sent.status, MessageStatus::Pending);
        assert_eq!(sent.received_at_ms, None);
        assert_eq!(
            (sent.sent_at_ms, sent.body.as_str(), sent.peer),
            (at, body, p)
        );
        assert_eq!(store.get(&a, &p, &sent.id).unwrap(), Some(sent));
    }
    let got = incoming(store, &a, &p, -7, body);
    assert_eq!(got.direction, MessageDirection::Incoming);
    assert_eq!(got.status, MessageStatus::Received);
    assert_eq!(got.received_at_ms, Some(5));
    assert_eq!(store.get(&a, &p, &got.id).unwrap(), Some(got));
    assert_eq!(store.get(&a, &p, &new_message_id()).unwrap(), None);
}

fn duplicates_are_detected(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let msg = chat(1, "first");
    let Insert::Inserted(row) = store.insert_incoming(&a, &p, &msg, 9).unwrap() else {
        panic!("first insert must store");
    };
    let again = ChatMessage {
        body: "changed".into(),
        ..msg.clone()
    };
    assert_eq!(
        store.insert_incoming(&a, &p, &again, 10).unwrap(),
        Insert::Duplicate
    );
    assert_eq!(store.page(&a, &p, None, 10).unwrap(), vec![row]);

    let sent = chat(2, "mine");
    store
        .insert_outgoing(&a, &p, &sent, MessageStatus::Sending)
        .unwrap();
    assert_eq!(
        store.insert_outgoing(&a, &p, &sent, MessageStatus::Sending),
        Err(MessageStoreError::AlreadyExists)
    );
    // An incoming frame colliding with our own outgoing id is dropped.
    assert_eq!(
        store.insert_incoming(&a, &p, &sent, 11).unwrap(),
        Insert::Duplicate
    );
    let kept = store.get(&a, &p, &sent.id).unwrap().unwrap();
    assert_eq!(
        (kept.direction, kept.status),
        (MessageDirection::Outgoing, MessageStatus::Sending)
    );
    assert_eq!(store.page(&a, &p, None, 10).unwrap().len(), 2);
}

fn status_lifecycle(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let row = out(store, &a, &p, 1, "x");
    let set = |s| store.set_status(&a, &p, &row.id, s).unwrap();
    let status = || store.get(&a, &p, &row.id).unwrap().unwrap().status;
    assert!(set(MessageStatus::Pending));
    assert_eq!(status(), MessageStatus::Pending);
    assert!(set(MessageStatus::Delivered));
    assert!(!set(MessageStatus::Failed));
    assert_eq!(status(), MessageStatus::Delivered);
    assert_eq!(
        store.set_status(&a, &p, &row.id, MessageStatus::Received),
        Err(MessageStoreError::InvalidQuery)
    );

    let other = out(store, &a, &p, 2, "y");
    assert!(store
        .set_status(&a, &p, &other.id, MessageStatus::Failed)
        .unwrap());
    assert!(store
        .set_status(&a, &p, &other.id, MessageStatus::Sending)
        .unwrap());

    let inc = incoming(store, &a, &p, 3, "z");
    assert!(!store
        .set_status(&a, &p, &inc.id, MessageStatus::Delivered)
        .unwrap());
    assert_eq!(store.get(&a, &p, &inc.id).unwrap(), Some(inc));
    assert!(!store
        .set_status(&a, &p, &new_message_id(), MessageStatus::Failed)
        .unwrap());
}

fn five_rows(store: &dyn MessageStore, a: &AccountId, p: &NodeId) -> Vec<StoredMessage> {
    [300, 100, 200, 100, 100]
        .iter()
        .map(|at| out(store, a, p, *at, "row"))
        .collect()
}

fn page_orders_by_sent_at_then_seq(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let r = five_rows(store, &a, &p);
    let expected = vec![r[1].id, r[3].id, r[4].id, r[2].id, r[0].id];
    assert_eq!(ids(&store.page(&a, &p, None, 10).unwrap()), expected);
    let before = HistoryCursor::of(&r[2]);
    assert_eq!(
        ids(&store.page(&a, &p, Some(before), 10).unwrap()),
        expected[..3].to_vec()
    );
}

fn keyset_paging_has_no_gaps_or_duplicates(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let r = five_rows(store, &a, &p);
    let expected = vec![r[1].id, r[3].id, r[4].id, r[2].id, r[0].id];
    let mut collected: Vec<protocol::MessageId> = Vec::new();
    let mut before = None;
    let mut pages = 0;
    loop {
        let page = history_page(store, &a, &p, before, Some(2)).unwrap();
        pages += 1;
        let mut older = ids(&page.messages);
        older.append(&mut collected);
        collected = older;
        match page.next_before {
            Some(c) => before = Some(c),
            None => {
                assert!(page.messages.len() < 2);
                break;
            }
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(collected, expected);
    let full = history_page(store, &a, &p, None, Some(5)).unwrap();
    assert_eq!(full.messages.len(), 5);
    assert!(full.next_before.is_some());
}

fn undelivered_is_bounded_and_ordered(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let first = out(store, &a, &p, 50, "a");
    let second = store
        .insert_outgoing(&a, &p, &chat(10, "b"), MessageStatus::Pending)
        .unwrap();
    let failed = out(store, &a, &p, 1, "c");
    store
        .set_status(&a, &p, &failed.id, MessageStatus::Failed)
        .unwrap();
    let delivered = out(store, &a, &p, 2, "d");
    store
        .set_status(&a, &p, &delivered.id, MessageStatus::Delivered)
        .unwrap();
    incoming(store, &a, &p, 3, "e");
    assert_eq!(
        ids(&store.undelivered(&a, usize::MAX).unwrap()),
        vec![first.id, second.id]
    );
    assert_eq!(ids(&store.undelivered(&a, 1).unwrap()), vec![first.id]);
}

fn limits_are_clamped(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    for i in 0..(MAX_PAGE + 5) {
        out(store, &a, &p, i64::try_from(i).unwrap(), "clamp me");
    }
    assert_eq!(
        store.page(&a, &p, None, usize::MAX).unwrap().len(),
        MAX_PAGE
    );
    assert_eq!(store.page(&a, &p, None, 0).unwrap().len(), 1);
    let sent = store.search(&a, None, "clamp", usize::MAX).unwrap();
    assert_eq!(sent.len(), MAX_SEARCH_RESULTS);
    assert_eq!(store.search(&a, None, "clamp", 0).unwrap().len(), 1);
    let default = history_page(store, &a, &p, None, None).unwrap();
    assert_eq!(default.messages.len(), 50);
    let huge = history_page(store, &a, &p, None, Some(usize::MAX)).unwrap();
    assert_eq!(huge.messages.len(), MAX_PAGE);
}

fn search_behaviour(store: &dyn MessageStore) {
    let (a, p, q) = (AccountId::new(), node(), node());
    let hello = out(store, &a, &p, 1, "HELLO wide World");
    let other = incoming(store, &a, &q, 2, "hello again");
    let hits =
        |peer: Option<&NodeId>, query: &str| ids(&store.search(&a, peer, query, 50).unwrap());

    assert_eq!(hits(None, "wide"), vec![hello.id]);
    assert_eq!(hits(None, "hello"), vec![other.id, hello.id]);
    assert_eq!(hits(None, "WORLD"), vec![hello.id]);
    assert_eq!(hits(None, "  wide  "), vec![hello.id]);
    assert_eq!(hits(None, "he"), vec![other.id, hello.id]);
    assert_eq!(hits(None, "Z"), Vec::<protocol::MessageId>::new());
    assert_eq!(hits(Some(&p), "hello"), vec![hello.id]);
    assert_eq!(hits(Some(&q), "hello"), vec![other.id]);
    assert_eq!(
        ids(&store.search(&a, None, "hello", 1).unwrap()),
        vec![other.id]
    );

    for bad in ["", "   ", &"x".repeat(129)] {
        assert_eq!(
            store.search(&a, None, bad, 10),
            Err(MessageStoreError::InvalidQuery),
            "{} chars",
            bad.len()
        );
    }
    assert!(store
        .search(&a, None, &"x".repeat(128), 10)
        .unwrap()
        .is_empty());

    let intl = out(store, &a, &p, 3, "Grüße aus 日本 👋");
    for q in ["üße", "日本", "👋", "Grüße aus"] {
        assert_eq!(hits(None, q), vec![intl.id], "{q}");
    }
}

fn search_treats_sql_and_fts_syntax_as_text(store: &dyn MessageStore) {
    let (a, p) = (AccountId::new(), node());
    let bodies = [
        "100%_done",
        "say \"quoted\" text",
        "it's x -- y",
        "back\\slash",
        "alpha",
        "beta NOT alpha",
    ];
    let rows: Vec<_> = bodies.iter().map(|b| out(store, &a, &p, 1, b)).collect();
    let hits = |query: &str| ids(&store.search(&a, None, query, 50).unwrap());
    assert_eq!(hits("%"), vec![rows[0].id]);
    assert_eq!(hits("_"), vec![rows[0].id]);
    assert_eq!(hits("0%_"), vec![rows[0].id]);
    assert_eq!(hits("\""), vec![rows[1].id]);
    assert_eq!(hits("\"quoted\""), vec![rows[1].id]);
    assert_eq!(hits("'"), vec![rows[2].id]);
    assert_eq!(hits("--"), vec![rows[2].id]);
    assert_eq!(hits("x -- y"), vec![rows[2].id]);
    assert_eq!(hits("\\"), vec![rows[3].id]);
    assert_eq!(hits("NOT alpha"), vec![rows[5].id]);
    assert!(hits("alpha OR beta").is_empty());
    assert!(hits("al*").is_empty());
    assert!(hits("'; DROP TABLE messages; --").is_empty());
    assert_eq!(store.page(&a, &p, None, 50).unwrap().len(), bodies.len());
}

fn accounts_are_isolated(store: &dyn MessageStore) {
    let (a, b, p) = (AccountId::new(), AccountId::new(), node());
    let msg = chat(1, "shared secret");
    store
        .insert_outgoing(&a, &p, &msg, MessageStatus::Sending)
        .unwrap();
    // Same peer and id under another account is a distinct row.
    store
        .insert_outgoing(&b, &p, &msg, MessageStatus::Sending)
        .unwrap();
    let only_b = out(store, &b, &p, 2, "only b secret");

    assert_eq!(store.page(&a, &p, None, 10).unwrap().len(), 1);
    assert_eq!(store.search(&a, None, "secret", 10).unwrap().len(), 1);
    assert_eq!(store.undelivered(&a, 10).unwrap().len(), 1);
    assert_eq!(store.get(&a, &p, &only_b.id).unwrap(), None);
    assert!(!store
        .set_status(&a, &p, &only_b.id, MessageStatus::Failed)
        .unwrap());

    store.erase_peer_history(&a, &p).unwrap();
    assert!(store.page(&a, &p, None, 10).unwrap().is_empty());
    assert_eq!(store.page(&b, &p, None, 10).unwrap().len(), 2);
    assert_eq!(store.search(&b, None, "secret", 10).unwrap().len(), 2);
}

fn erase_removes_only_that_peer(store: &dyn MessageStore) {
    let (a, p, q) = (AccountId::new(), node(), node());
    out(store, &a, &p, 1, "erase me please");
    incoming(store, &a, &p, 2, "erase me too");
    let kept = out(store, &a, &q, 3, "erase me not");
    store.erase_peer_history(&a, &p).unwrap();
    store.erase_peer_history(&a, &p).unwrap();
    assert!(store.page(&a, &p, None, 10).unwrap().is_empty());
    assert_eq!(
        ids(&store.search(&a, None, "erase me", 10).unwrap()),
        vec![kept.id]
    );
    assert_eq!(store.page(&a, &q, None, 10).unwrap(), vec![kept]);
}
