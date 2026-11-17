//! Level-1 messaging engine tests: real iroh endpoints on loopback, a contact
//! book as the trust store, and the delivery engine on top. The package name
//! `core` shadows std `core`, which breaks `#[tokio::test]`, so tests run
//! through `run`.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use core::contacts::HistoryChoice;
use core::discovery::testing::{next_event, TestNode};
use core::discovery::NetEvent;
use core::identity::NodeId;
use core::messaging::testing::{
    chat_peer, connect_and_accept, restart_chat_peer, send_raw_frame, ChatPeer, StoreKind,
};
use core::messaging::{
    encode_chat, history_page, ChatEvent, MessageDirection, MessageStatus, MessagingConfig,
    SendError, StoredMessage, MAX_BODY_BYTES,
};
use iroh::endpoint::{ReadToEndError, RecvStream};
use protocol::{new_message_id, ChatMessage, MessageId, MAX_ACK_BYTES};
use tokio::sync::broadcast::Receiver;

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

const WAIT: Duration = Duration::from_secs(10);

use StoreKind::{Memory, SqlCipher};

async fn peer(name: &str, kind: StoreKind) -> ChatPeer {
    chat_peer(name, kind, MessagingConfig::fast()).await
}

async fn peer_with(name: &str, config: MessagingConfig) -> ChatPeer {
    chat_peer(name, Memory, config).await
}

async fn pair(kind_a: StoreKind, kind_b: StoreKind) -> (ChatPeer, ChatPeer) {
    let (a, b) = (peer("Alice", kind_a).await, peer("Bob", kind_b).await);
    connect_and_accept(&a, &b).await;
    (a, b)
}

fn stranger() -> NodeId {
    iroh::SecretKey::generate().public()
}

async fn until(what: &str, mut probe: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !probe() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn wall_ms() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(d.as_millis()).unwrap()
}

fn row(p: &ChatPeer, peer: &NodeId, id: &MessageId) -> Option<StoredMessage> {
    p.store.get(&p.node.node.account(), peer, id).unwrap()
}

fn history(p: &ChatPeer, peer: &NodeId) -> Vec<StoredMessage> {
    history_page(
        p.store.as_ref(),
        &p.node.node.account(),
        peer,
        None,
        Some(200),
    )
    .unwrap()
    .messages
}

async fn wait_status(p: &ChatPeer, peer: &NodeId, id: &MessageId, want: MessageStatus) {
    until(&format!("status {want:?}"), || {
        row(p, peer, id).is_some_and(|r| r.status == want)
    })
    .await;
}

async fn gone(p: &ChatPeer, other: &ChatPeer) {
    until("channel down", || {
        p.node.node.channel(&other.node.id()).is_none()
            && other.node.node.channel(&p.node.id()).is_none()
    })
    .await;
}

async fn disconnect(a: &ChatPeer, b: &ChatPeer) {
    a.node.node.disconnect(&b.node.id());
    gone(a, b).await;
}

async fn next_chat(rx: &mut Receiver<ChatEvent>, within: Duration) -> Option<ChatEvent> {
    tokio::time::timeout(within, rx.recv()).await.ok()?.ok()
}

fn is_status(ev: &ChatEvent, want: MessageStatus, want_id: &MessageId) -> bool {
    matches!(ev, ChatEvent::Status { id, status, .. } if id == want_id && *status == want)
}

async fn count_received(rx: &mut Receiver<ChatEvent>, window: Duration) -> usize {
    let mut n = 0;
    while let Some(ev) = next_chat(rx, window).await {
        n += usize::from(matches!(ev, ChatEvent::Received(_)));
    }
    n
}

fn frame(id: MessageId, sent_at_ms: i64, body: &str) -> Vec<u8> {
    encode_chat(&ChatMessage {
        id,
        sent_at_ms,
        body: body.to_owned(),
    })
    .unwrap()
}

fn insert_undelivered(
    p: &ChatPeer,
    to: &NodeId,
    sent_at_ms: i64,
    status: MessageStatus,
) -> MessageId {
    let msg = ChatMessage {
        id: new_message_id(),
        sent_at_ms,
        body: format!("queued {sent_at_ms}"),
    };
    p.store
        .insert_outgoing(&p.node.node.account(), to, &msg, status)
        .unwrap();
    msg.id
}

#[test]
fn happy_path_sending_then_delivered() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (mut ea, mut eb) = (a.messaging.subscribe(), b.messaging.subscribe());
        let body = "hi 👋\nsecond line";
        let sent = a
            .messaging
            .send(b.node.id(), body.to_owned())
            .await
            .unwrap();
        assert_eq!(sent.status, MessageStatus::Sending);

        let Some(ChatEvent::Received(got)) = next_chat(&mut eb, WAIT).await else {
            panic!("no Received event");
        };
        assert_eq!(
            (got.id, got.body.as_str(), got.sent_at_ms),
            (sent.id, body, sent.sent_at_ms)
        );
        assert_eq!(got.direction, MessageDirection::Incoming);
        assert_eq!(count_received(&mut eb, Duration::from_millis(200)).await, 0);
        assert_eq!(history(&b, &a.node.id()).len(), 1);

        let at = loop {
            let ev = next_chat(&mut ea, WAIT).await.expect("no Delivered event");
            if is_status(&ev, MessageStatus::Delivered, &sent.id) {
                let ChatEvent::Status {
                    delivered_at_ms, ..
                } = ev
                else {
                    unreachable!()
                };
                break delivered_at_ms.expect("Delivered carries a time");
            }
        };
        assert!(sent.sent_at_ms <= at && at <= wall_ms());
        let stored = row(&a, &b.node.id(), &sent.id).unwrap();
        assert_eq!(stored.status, MessageStatus::Delivered);
        assert_eq!(stored.delivered_at_ms, Some(at));
    });
}

#[test]
fn duplicate_chat_frame_stores_once_and_acks_twice() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let mut eb = b.messaging.subscribe();
        let id = new_message_id();
        let bytes = frame(id, 1_000, "once");
        for _ in 0..2 {
            let reply = send_raw_frame(&a.node.node, &b.node.id(), &bytes).await;
            assert_eq!(reply.ack, Some(id));
        }
        assert_eq!(b.config.faults.acks_sent.load(Ordering::SeqCst), 2);
        assert_eq!(history(&b, &a.node.id()).len(), 1);
        assert_eq!(count_received(&mut eb, Duration::from_millis(300)).await, 1);
    });
}

#[test]
fn lost_ack_is_retried_with_same_id_and_ends_delivered() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let mut eb = b.messaging.subscribe();
        b.config.faults.drop_acks.store(1, Ordering::SeqCst);
        let sent = a
            .messaging
            .send(b.node.id(), "retry me".into())
            .await
            .unwrap();
        wait_status(&a, &b.node.id(), &sent.id, MessageStatus::Delivered).await;
        let rows = history(&b, &a.node.id());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, sent.id);
        assert_eq!(count_received(&mut eb, Duration::from_millis(300)).await, 1);
    });
}

#[test]
fn lost_ack_records_one_delivered_time() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (a_acc, b_id) = (a.node.node.account(), b.node.id());
        let mut ea = a.messaging.subscribe();
        b.config.faults.drop_acks.store(1, Ordering::SeqCst);
        let sent = a
            .messaging
            .send(b_id, "retry me once".into())
            .await
            .unwrap();
        wait_status(&a, &b_id, &sent.id, MessageStatus::Delivered).await;
        let mut times = Vec::new();
        while let Some(ev) = next_chat(&mut ea, Duration::from_millis(300)).await {
            if let ChatEvent::Status {
                id,
                status: MessageStatus::Delivered,
                delivered_at_ms,
                ..
            } = ev
            {
                if id == sent.id {
                    times.push(delivered_at_ms);
                }
            }
        }
        assert_eq!(times.len(), 1);
        let stored = row(&a, &b_id, &sent.id).unwrap();
        assert!(stored.delivered_at_ms.is_some());
        assert_eq!(times[0], stored.delivered_at_ms);
        let other = stored.delivered_at_ms.unwrap() + 1_000;
        assert!(!a
            .store
            .mark_delivered(&a_acc, &b_id, &sent.id, other)
            .unwrap());
        assert_eq!(row(&a, &b_id, &sent.id).unwrap(), stored);
    });
}

#[test]
fn send_to_offline_peer_stays_pending_and_survives_reload() {
    run(async {
        let (a, b) = pair(SqlCipher, Memory).await;
        let b_id = b.node.id();
        b.messaging.shutdown();
        b.node.node.shutdown().await;
        until("channel down", || a.node.node.channel(&b_id).is_none()).await;

        let sent = a.messaging.send(b_id, "waiting".into()).await.unwrap();
        assert_eq!(sent.status, MessageStatus::Pending);
        let patience = a.config.backoff_max * 5;
        tokio::time::sleep(patience).await;
        assert_eq!(
            row(&a, &b_id, &sent.id).unwrap().status,
            MessageStatus::Pending
        );

        let a = restart_chat_peer(a).await;
        let rows = history(&a, &b_id);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].id, rows[0].body.as_str()), (sent.id, "waiting"));
        assert_eq!(rows[0].status, MessageStatus::Pending);
        tokio::time::sleep(patience).await;
        assert_eq!(
            row(&a, &b_id, &sent.id).unwrap().status,
            MessageStatus::Pending
        );
    });
}

#[test]
fn pending_messages_deliver_after_channel_restore() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let b_id = b.node.id();
        disconnect(&a, &b).await;
        let mut ids = HashSet::new();
        for i in 0..3 {
            let sent = a.messaging.send(b_id, format!("m{i}")).await.unwrap();
            assert_eq!(sent.status, MessageStatus::Pending);
            ids.insert(sent.id);
        }
        connect_and_accept(&a, &b).await;
        for id in &ids {
            wait_status(&a, &b_id, id, MessageStatus::Delivered).await;
        }
        let rows = history(&b, &a.node.id());
        assert_eq!(rows.iter().map(|r| r.id).collect::<HashSet<_>>(), ids);
        assert_eq!(rows.len(), 3);
        assert!(rows
            .windows(2)
            .all(|w| (w[0].sent_at_ms, w[0].seq) <= (w[1].sent_at_ms, w[1].seq)));
    });
}

#[test]
fn retries_exhaust_to_failed_then_manual_resend_reuses_id() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (a_id, b_id) = (a.node.id(), b.node.id());
        let mut ea = a.messaging.subscribe();
        b.config
            .faults
            .drop_acks
            .store(usize::MAX, Ordering::SeqCst);
        let sent = a.messaging.send(b_id, "stubborn".into()).await.unwrap();
        assert_eq!(sent.status, MessageStatus::Sending);
        let mut saw_pending = false;
        loop {
            let ev = next_chat(&mut ea, WAIT).await.expect("no Failed event");
            saw_pending |= is_status(&ev, MessageStatus::Pending, &sent.id);
            if is_status(&ev, MessageStatus::Failed, &sent.id) {
                assert!(matches!(
                    ev,
                    ChatEvent::Status {
                        delivered_at_ms: None,
                        ..
                    }
                ));
                break;
            }
        }
        assert!(saw_pending);
        wait_status(&a, &b_id, &sent.id, MessageStatus::Failed).await;
        assert_eq!(row(&a, &b_id, &sent.id).unwrap().delivered_at_ms, None);

        b.config.faults.drop_acks.store(0, Ordering::SeqCst);
        let status = a.messaging.resend(b_id, sent.id).await.unwrap();
        assert_eq!(status, MessageStatus::Sending);
        wait_status(&a, &b_id, &sent.id, MessageStatus::Delivered).await;
        let rows = history(&b, &a_id);
        assert_eq!(rows.iter().filter(|r| r.id == sent.id).count(), 1);
        let original = row(&a, &b_id, &sent.id).unwrap();
        assert_eq!(original.sent_at_ms, sent.sent_at_ms);
        assert!(original
            .delivered_at_ms
            .is_some_and(|t| t >= sent.sent_at_ms));

        assert_eq!(
            a.messaging.resend(b_id, sent.id).await,
            Err(SendError::NotResendable)
        );
        assert_eq!(
            a.messaging.resend(b_id, new_message_id()).await,
            Err(SendError::NotFound)
        );
    });
}

#[test]
fn restart_reload_resumes_pending_and_marks_overflow_failed() {
    run(async {
        let config = MessagingConfig {
            max_outbox: 2,
            ..MessagingConfig::fast()
        };
        let (a, b) = (
            peer_with("Alice", config.clone()).await,
            peer_with("Bob", MessagingConfig::fast()).await,
        );
        connect_and_accept(&a, &b).await;
        let b_id = b.node.id();
        disconnect(&a, &b).await;
        let first = insert_undelivered(&a, &b_id, 1_000, MessageStatus::Sending);
        let second = insert_undelivered(&a, &b_id, 2_000, MessageStatus::Pending);
        let third = insert_undelivered(&a, &b_id, 3_000, MessageStatus::Pending);

        let a = restart_chat_peer(a).await;
        assert_eq!(
            row(&a, &b_id, &first).unwrap().status,
            MessageStatus::Pending
        );
        assert_eq!(
            row(&a, &b_id, &second).unwrap().status,
            MessageStatus::Pending
        );
        assert_eq!(
            row(&a, &b_id, &third).unwrap().status,
            MessageStatus::Failed
        );

        connect_and_accept(&a, &b).await;
        wait_status(&a, &b_id, &first, MessageStatus::Delivered).await;
        wait_status(&a, &b_id, &second, MessageStatus::Delivered).await;
        assert_eq!(
            row(&a, &b_id, &third).unwrap().status,
            MessageStatus::Failed
        );
        assert_eq!(history(&b, &a.node.id()).len(), 2);
    });
}

#[test]
fn reload_fails_rows_for_unknown_contacts() {
    run(async {
        let config = MessagingConfig {
            max_outbox: 2,
            ..MessagingConfig::fast()
        };
        let a = peer_with("Alice", config).await;
        let unknown = stranger();
        let ids: Vec<_> = (1..=3)
            .map(|i| insert_undelivered(&a, &unknown, i * 1_000, MessageStatus::Pending))
            .collect();
        let a = restart_chat_peer(a).await;
        for id in &ids {
            assert_eq!(row(&a, &unknown, id).unwrap().status, MessageStatus::Failed);
        }
        let contact = stranger();
        a.book.get().add(contact, "carol", None).unwrap();
        for i in 0..2 {
            let sent = a.messaging.send(contact, format!("m{i}")).await.unwrap();
            assert_eq!(sent.status, MessageStatus::Pending);
        }
        assert_eq!(
            a.messaging
                .send(contact, "one too many".into())
                .await
                .map(|_| ()),
            Err(SendError::OutboxFull)
        );
    });
}

#[test]
fn removing_contact_fails_pending_and_frees_slots() {
    run(async {
        let config = MessagingConfig {
            max_outbox: 2,
            ..MessagingConfig::fast()
        };
        let (a, b) = (
            peer_with("Alice", config).await,
            peer_with("Bob", MessagingConfig::fast()).await,
        );
        connect_and_accept(&a, &b).await;
        let b_id = b.node.id();
        let c = stranger();
        a.book.get().add(c, "carol", None).unwrap();
        disconnect(&a, &b).await;
        let mut ea = a.messaging.subscribe();
        let mut ids = Vec::new();
        for i in 0..2 {
            let sent = a.messaging.send(b_id, format!("m{i}")).await.unwrap();
            assert_eq!(sent.status, MessageStatus::Pending);
            ids.push(sent.id);
        }
        assert_eq!(
            a.messaging.send(c, "full".into()).await.map(|_| ()),
            Err(SendError::OutboxFull)
        );

        a.book.get().remove(&b_id, HistoryChoice::Keep).unwrap();
        a.messaging.forget_peer(b_id).await;
        for id in &ids {
            assert_eq!(row(&a, &b_id, id).unwrap().status, MessageStatus::Failed);
        }
        let mut failed = 0;
        while let Some(ev) = next_chat(&mut ea, Duration::from_millis(200)).await {
            failed += ids
                .iter()
                .filter(|id| is_status(&ev, MessageStatus::Failed, id))
                .count();
        }
        assert_eq!(failed, 2);
        assert!(a.messaging.send(c, "room again".into()).await.is_ok());
    });
}

#[test]
fn removing_contact_without_forget_fails_on_driver_wake() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (a_id, b_id) = (a.node.id(), b.node.id());
        disconnect(&a, &b).await;
        let ids: Vec<_> = {
            let mut ids = Vec::new();
            for i in 0..2 {
                ids.push(a.messaging.send(b_id, format!("m{i}")).await.unwrap().id);
            }
            ids
        };
        a.book.get().remove(&b_id, HistoryChoice::Keep).unwrap();

        let mut ea = a.node.node.subscribe();
        let ticket = a.node.node.ticket().unwrap().to_string();
        b.node.node.connect_ticket(&ticket).unwrap();
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match next_event(&mut ea, left).await {
                Some(NetEvent::IncomingRequest(_)) => break,
                Some(_) => {}
                None => panic!("no prompt on a"),
            }
        }
        a.node.node.respond_incoming(&b_id, true).unwrap();
        for id in &ids {
            wait_status(&a, &b_id, id, MessageStatus::Failed).await;
        }
        assert!(history(&b, &a_id).is_empty());
    });
}

#[test]
fn outbox_full_rejects_without_persisting() {
    run(async {
        let config = MessagingConfig {
            max_outbox: 2,
            ..MessagingConfig::fast()
        };
        let a = peer_with("Alice", config).await;
        let contact = stranger();
        a.book.get().add(contact, "carol", None).unwrap();
        for i in 0..2 {
            a.messaging.send(contact, format!("m{i}")).await.unwrap();
        }
        assert_eq!(
            a.messaging
                .send(contact, "rejected".into())
                .await
                .map(|_| ()),
            Err(SendError::OutboxFull)
        );
        assert_eq!(history(&a, &contact).len(), 2);
    });
}

#[test]
fn in_flight_per_peer_is_bounded() {
    run(async {
        let config = MessagingConfig {
            max_in_flight_per_peer: 2,
            ..MessagingConfig::fast()
        };
        let (a, b) = (
            peer_with("Alice", config.clone()).await,
            peer_with("Bob", MessagingConfig::fast()).await,
        );
        connect_and_accept(&a, &b).await;
        let b_id = b.node.id();
        b.config
            .faults
            .drop_acks
            .store(usize::MAX, Ordering::SeqCst);
        let mut ids = Vec::new();
        for i in 0..3 * config.max_in_flight_per_peer {
            ids.push(a.messaging.send(b_id, format!("m{i}")).await.unwrap().id);
        }
        for id in &ids {
            wait_status(&a, &b_id, id, MessageStatus::Failed).await;
        }
        let peak = a
            .config
            .faults
            .outbound_in_flight_max
            .load(Ordering::SeqCst);
        assert!(
            (1..=config.max_in_flight_per_peer).contains(&peak),
            "{peak}"
        );
    });
}

#[test]
fn send_to_non_contact_is_refused() {
    run(async {
        let a = peer("Alice", Memory).await;
        let unknown = stranger();
        assert_eq!(
            a.messaging.send(unknown, "hello?".into()).await.map(|_| ()),
            Err(SendError::NotAContact)
        );
        assert!(history(&a, &unknown).is_empty());
    });
}

#[test]
fn empty_or_whitespace_body_is_not_sent() {
    run(async {
        let a = peer("Alice", Memory).await;
        let contact = stranger();
        a.book.get().add(contact, "carol", None).unwrap();
        for body in ["", "   ", " \t\r\n ", "\r"] {
            assert_eq!(
                a.messaging.send(contact, body.into()).await.map(|_| ()),
                Err(SendError::EmptyBody)
            );
        }
        assert!(history(&a, &contact).is_empty());
    });
}

#[test]
fn oversized_body_rejected_on_send() {
    run(async {
        let a = peer("Alice", Memory).await;
        let contact = stranger();
        a.book.get().add(contact, "carol", None).unwrap();
        assert_eq!(
            a.messaging
                .send(contact, "a".repeat(MAX_BODY_BYTES + 1))
                .await
                .map(|_| ()),
            Err(SendError::BodyTooLarge)
        );
        assert!(history(&a, &contact).is_empty());
        let at_limit = a.messaging.send(contact, "a".repeat(MAX_BODY_BYTES)).await;
        assert!(at_limit.is_ok());
    });
}

#[test]
fn self_message_is_refused() {
    run(async {
        let a = peer("Alice", Memory).await;
        assert_eq!(
            a.messaging.send(a.node.id(), "me".into()).await.map(|_| ()),
            Err(SendError::SelfMessage)
        );
    });
}

#[test]
fn removed_contact_cannot_deliver_or_receive() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (a_id, b_id) = (a.node.id(), b.node.id());
        let mut eb = b.messaging.subscribe();
        b.book.get().remove(&a_id, HistoryChoice::Keep).unwrap();
        let sent = a.messaging.send(b_id, "unwanted".into()).await.unwrap();
        wait_status(&a, &b_id, &sent.id, MessageStatus::Failed).await;
        assert!(history(&b, &a_id).is_empty());
        assert_eq!(count_received(&mut eb, Duration::from_millis(200)).await, 0);

        a.book.get().remove(&b_id, HistoryChoice::Keep).unwrap();
        assert_eq!(
            a.messaging.send(b_id, "again".into()).await.map(|_| ()),
            Err(SendError::NotAContact)
        );
    });
}

#[test]
fn persistence_round_trip_with_sqlcipher() {
    run(async {
        let (a, b) = pair(SqlCipher, SqlCipher).await;
        let (a_id, b_id) = (a.node.id(), b.node.id());
        let mut sent = Vec::new();
        sent.push(
            a.messaging
                .send(b_id, "one 👋\nline".into())
                .await
                .unwrap()
                .id,
        );
        sent.push(a.messaging.send(b_id, "two".into()).await.unwrap().id);
        let reply = b.messaging.send(a_id, "reply".into()).await.unwrap().id;
        for id in &sent {
            wait_status(&a, &b_id, id, MessageStatus::Delivered).await;
        }
        wait_status(&b, &a_id, &reply, MessageStatus::Delivered).await;
        until("both sides stored", || {
            history(&a, &b_id).len() == 3 && history(&b, &a_id).len() == 3
        })
        .await;
        let (before_a, before_b) = (history(&a, &b_id), history(&b, &a_id));

        let (a, b) = (restart_chat_peer(a).await, restart_chat_peer(b).await);
        let (after_a, after_b) = (history(&a, &b_id), history(&b, &a_id));
        assert_eq!(after_a, before_a);
        assert_eq!(after_b, before_b);
        for r in &after_a {
            let outgoing = r.direction == MessageDirection::Outgoing;
            assert_eq!(r.status == MessageStatus::Delivered, outgoing);
            assert_eq!(r.status == MessageStatus::Received, !outgoing);
            assert_eq!(r.received_at_ms.is_some(), !outgoing);
            assert_eq!(r.delivered_at_ms.is_some(), outgoing);
        }
        assert!(after_a.iter().any(|r| r.body == "one 👋\nline"));
    });
}

#[test]
fn history_orders_by_sender_time_then_arrival() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (x, y, z) = (new_message_id(), new_message_id(), new_message_id());
        for (id, at) in [(x, 2_000), (y, 1_000), (z, 1_000)] {
            let reply = send_raw_frame(&a.node.node, &b.node.id(), &frame(id, at, "t")).await;
            assert_eq!(reply.ack, Some(id));
        }
        let order: Vec<_> = history(&b, &a.node.id()).iter().map(|r| r.id).collect();
        assert_eq!(order, vec![y, z, x]);
    });
}

#[test]
fn inbound_persist_follows_accept_order() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let channel = a.node.node.channel(&b.node.id()).unwrap();
        let ids: Vec<_> = (0..3).map(|_| new_message_id()).collect();
        let mut streams = Vec::new();
        for _ in 0..3 {
            streams.push(channel.open_stream().await.unwrap());
        }
        for (i, (send, _)) in streams.iter_mut().enumerate().rev() {
            send.write_all(&frame(ids[i], 1_000, "same time"))
                .await
                .unwrap();
            send.finish().unwrap();
        }
        for ((_, recv), id) in streams.iter_mut().zip(&ids) {
            assert_eq!(read_ack(recv).await, Some(*id));
        }
        let order: Vec<_> = history(&b, &a.node.id()).iter().map(|r| r.id).collect();
        assert_eq!(order, ids);
    });
}

async fn read_ack(recv: &mut RecvStream) -> Option<MessageId> {
    let reply = tokio::time::timeout(WAIT, recv.read_to_end(MAX_ACK_BYTES)).await;
    let bytes: Result<Vec<u8>, ReadToEndError> = reply.ok()?;
    core::messaging::decode_ack(&bytes.ok()?).ok()
}

#[test]
fn incoming_id_collision_does_not_change_outgoing_status() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let (a_id, b_id) = (a.node.id(), b.node.id());
        let mut ea = a.messaging.subscribe();
        b.config
            .faults
            .drop_acks
            .store(usize::MAX, Ordering::SeqCst);
        let sent = a.messaging.send(b_id, "mine".into()).await.unwrap();
        wait_status(&a, &b_id, &sent.id, MessageStatus::Failed).await;
        while next_chat(&mut ea, Duration::from_millis(100))
            .await
            .is_some()
        {}

        let forged = frame(sent.id, 5, "forged");
        let reply = send_raw_frame(&b.node.node, &a_id, &forged).await;
        assert_eq!(reply.ack, Some(sent.id));
        let after = row(&a, &b_id, &sent.id).unwrap();
        assert_eq!(after.status, MessageStatus::Failed);
        assert_eq!(after.direction, MessageDirection::Outgoing);
        assert_eq!(after.body, "mine");
        assert_eq!(count_received(&mut ea, Duration::from_millis(300)).await, 0);
    });
}

#[test]
fn messaging_does_not_keep_node_alive() {
    run(async {
        let (a, b) = pair(Memory, Memory).await;
        let channel = a.node.node.channel(&b.node.id()).unwrap();
        assert!(channel.is_current());
        let ChatPeer {
            node, messaging, ..
        } = a;
        messaging.shutdown();
        let TestNode { node, .. } = node;
        let node = Arc::new(node);
        let node = Arc::try_unwrap(node)
            .ok()
            .expect("messaging holds no Arc<Node>");
        tokio::time::timeout(Duration::from_secs(2), node.shutdown())
            .await
            .expect("node shutdown completes");
        assert!(!channel.is_current());
    });
}
