//! Adversarial inbound frames from an authorized peer (`send_raw_frame`) and
//! from peers that never passed the trust gate (`raw_dial`).

use std::sync::atomic::Ordering;
use std::time::Duration;

use core::discovery::testing::{hello_frame, next_event, raw_dial};
use core::discovery::{NetEvent, CLOSE_PROTOCOL};
use core::identity::{NodeId, Ticket};
use core::messaging::testing::{
    chat_peer, connect_and_accept, send_raw_frame, ChatPeer, StoreKind,
};
use core::messaging::{
    encode_chat, history_page, ChatEvent, MessagingConfig, StoredMessage, MAX_BODY_BYTES,
};
use iroh::endpoint::ConnectionError;
use protocol::{
    new_message_id, Ack, ChatMessage, Envelope, MessageId, Payload, MAX_FRAME_BYTES,
    PROTOCOL_VERSION,
};

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(600);

async fn pair_with(b_config: MessagingConfig) -> (ChatPeer, ChatPeer) {
    let a = chat_peer("Alice", StoreKind::Memory, MessagingConfig::fast()).await;
    let b = chat_peer("Bob", StoreKind::Memory, b_config).await;
    connect_and_accept(&a, &b).await;
    (a, b)
}

async fn pair() -> (ChatPeer, ChatPeer) {
    pair_with(MessagingConfig::fast()).await
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

fn valid_frame(id: MessageId) -> Vec<u8> {
    encode_chat(&ChatMessage {
        id,
        sent_at_ms: 1_000,
        body: "valid".into(),
    })
    .unwrap()
}

async fn assert_still_serving(a: &ChatPeer, b: &ChatPeer) {
    let id = new_message_id();
    let reply = send_raw_frame(&a.node.node, &b.node.id(), &valid_frame(id)).await;
    assert_eq!(reply.ack, Some(id));
    assert!(history(b, &a.node.id()).iter().any(|r| r.id == id));
}

async fn drain(rx: &mut tokio::sync::broadcast::Receiver<ChatEvent>) -> usize {
    let mut n = 0;
    while tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .is_ok()
    {
        n += 1;
    }
    n
}

#[test]
fn unsupported_version_frame_is_rejected_not_decoded() {
    run(async {
        let (a, b) = pair().await;
        let mut eb = b.messaging.subscribe();
        let future = Envelope {
            version: PROTOCOL_VERSION + 1,
            payload: Payload::Chat(ChatMessage {
                id: new_message_id(),
                sent_at_ms: 1,
                body: "from the future".into(),
            }),
        };
        let reply = send_raw_frame(&a.node.node, &b.node.id(), &future.encode().unwrap()).await;
        assert_eq!(reply.ack, None);
        assert!(!reply.connection_closed);
        assert!(history(&b, &a.node.id()).is_empty());
        assert_eq!(drain(&mut eb).await, 0);
        assert_still_serving(&a, &b).await;
    });
}

#[test]
fn unknown_payload_variant_is_ignored_and_connection_survives() {
    run(async {
        let (a, b) = pair().await;
        let reply = send_raw_frame(&a.node.node, &b.node.id(), &[0x01, 0x07, 0x00]).await;
        assert_eq!(reply.ack, None);
        assert!(!reply.connection_closed);
        assert!(history(&b, &a.node.id()).is_empty());
        assert_still_serving(&a, &b).await;
    });
}

#[test]
fn oversized_inbound_frame_closes_connection_without_storing() {
    run(async {
        let (a, b) = pair().await;
        let oversized = vec![0u8; MAX_FRAME_BYTES + 1];
        let reply = send_raw_frame(&a.node.node, &b.node.id(), &oversized).await;
        assert!(reply.connection_closed);
        assert_eq!(reply.ack, None);
        assert!(history(&b, &a.node.id()).is_empty());

        let channel_gone = async {
            while a.node.node.channel(&b.node.id()).is_some()
                || b.node.node.channel(&a.node.id()).is_some()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(WAIT, channel_gone).await.unwrap();
        connect_and_accept(&a, &b).await;
        assert_still_serving(&a, &b).await;
    });
}

#[test]
fn oversized_body_inside_a_fitting_frame_closes_connection() {
    run(async {
        let (a, b) = pair().await;
        let big = Envelope::new(Payload::Chat(ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1,
            body: "a".repeat(MAX_BODY_BYTES + 1),
        }))
        .encode()
        .unwrap();
        assert!(big.len() <= MAX_FRAME_BYTES);
        let reply = send_raw_frame(&a.node.node, &b.node.id(), &big).await;
        assert!(reply.connection_closed);
        assert!(history(&b, &a.node.id()).is_empty());
    });
}

#[test]
fn ack_for_unknown_id_is_ignored() {
    run(async {
        let (a, b) = pair().await;
        let ack = Envelope::new(Payload::Ack(Ack {
            id: new_message_id(),
        }))
        .encode()
        .unwrap();
        let reply = send_raw_frame(&a.node.node, &b.node.id(), &ack).await;
        assert_eq!(reply.ack, None);
        assert!(!reply.connection_closed);
        assert!(history(&b, &a.node.id()).is_empty());
        assert_still_serving(&a, &b).await;
    });
}

async fn raw_to(b: &ChatPeer, first: &[u8]) -> core::discovery::testing::RawDial {
    let addr = Ticket::parse(&b.node.node.ticket().unwrap().to_string())
        .unwrap()
        .addr;
    raw_dial(addr, first).await.unwrap()
}

#[test]
fn pending_peer_chat_stream_is_never_processed() {
    run(async {
        let b = chat_peer("Bob", StoreKind::Memory, MessagingConfig::fast()).await;
        let mut events = b.node.node.subscribe();
        let dial = raw_to(&b, &hello_frame("Mallory")).await;
        let req = loop {
            match next_event(&mut events, WAIT).await {
                Some(NetEvent::IncomingRequest(req)) => break req,
                Some(_) => {}
                None => panic!("no prompt"),
            }
        };

        let (mut send, mut recv) = dial.conn.open_bi().await.unwrap();
        let _ = send.write_all(&valid_frame(new_message_id())).await;
        let _ = send.finish();
        let reply = tokio::time::timeout(QUIET, recv.read_to_end(64)).await;
        assert!(reply.is_err(), "nothing may answer before the trust gate");
        assert!(history(&b, &req.node_id).is_empty());

        b.node.node.respond_incoming(&req.node_id, false).unwrap();
        tokio::time::sleep(QUIET).await;
        assert!(history(&b, &req.node_id).is_empty());
    });
}

#[test]
fn chat_as_first_frame_is_refused() {
    run(async {
        let b = chat_peer("Bob", StoreKind::Memory, MessagingConfig::fast()).await;
        let dial = raw_to(&b, &valid_frame(new_message_id())).await;
        match tokio::time::timeout(WAIT, dial.conn.closed())
            .await
            .unwrap()
        {
            ConnectionError::ApplicationClosed(close) => {
                assert_eq!(u64::from(close.error_code), u64::from(CLOSE_PROTOCOL));
            }
            other => panic!("unexpected close: {other:?}"),
        }
        assert!(b
            .store
            .undelivered(&b.node.node.account(), 10)
            .unwrap()
            .is_empty());
    });
}

#[test]
fn inbound_concurrency_is_bounded() {
    run(async {
        let cap = 4;
        let config = MessagingConfig {
            max_inbound_per_peer: cap,
            ..MessagingConfig::fast()
        };
        let (a, b) = pair_with(config).await;
        let channel = a.node.node.channel(&b.node.id()).unwrap();
        let mut held = Vec::new();
        for _ in 0..3 * cap {
            let (mut send, recv) = channel.open_stream().await.unwrap();
            send.write_all(&[0u8]).await.unwrap();
            held.push((send, recv));
        }
        let peak = &b.config.faults.inbound_in_flight_max;
        let deadline = tokio::time::Instant::now() + WAIT;
        while peak.load(Ordering::SeqCst) < cap {
            assert!(
                tokio::time::Instant::now() < deadline,
                "readers never filled"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(peak.load(Ordering::SeqCst) <= cap + 1);
        assert!(history(&b, &a.node.id()).is_empty());
    });
}
