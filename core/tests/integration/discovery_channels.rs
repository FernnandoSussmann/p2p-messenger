//! Level-1: the authorized-channel hook. Channels reach the handler only after
//! the Hello exchange and the trust gate, never earlier.

use std::sync::Arc;
use std::time::Duration;

use core::discovery::testing::{
    hello_frame, next_event, raw_dial, spawn_test_node_with, MemoryKnownPeers,
    RecordingChannelHandler, TestNode, TestOptions,
};
use core::discovery::{ConnectionState, NetEvent, Node, CLOSE_PROTOCOL};
use core::identity::{NodeId, Ticket};
use iroh::endpoint::ConnectionError;
use tokio::sync::broadcast::Receiver;

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(400);

async fn node(name: &str) -> TestNode {
    let options = TestOptions::default().with_known(Arc::new(MemoryKnownPeers::new()));
    spawn_test_node_with(name, options).await
}

async fn wait_for(rx: &mut Receiver<NetEvent>, pred: impl Fn(&NetEvent) -> bool) -> NetEvent {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(rx, left).await {
            Some(ev) if pred(&ev) => return ev,
            Some(_) => continue,
            None => panic!("timed out waiting for event"),
        }
    }
}

fn is_state(peer: NodeId, want: ConnectionState) -> impl Fn(&NetEvent) -> bool {
    move |ev| {
        matches!(ev, NetEvent::ConnectionState { node_id, state, .. }
            if *node_id == peer && *state == want)
    }
}

fn is_incoming(ev: &NetEvent) -> bool {
    matches!(ev, NetEvent::IncomingRequest(_))
}

/// `dialer` dials `acceptor`, which accepts; returns once both are Connected.
async fn connect(dialer: &Node, acceptor: &Node) {
    let (mut ed, mut ea) = (dialer.subscribe(), acceptor.subscribe());
    dialer
        .connect_ticket(&acceptor.ticket().unwrap().to_string())
        .unwrap();
    wait_for(&mut ea, is_incoming).await;
    acceptor.respond_incoming(&dialer.node_id(), true).unwrap();
    wait_for(
        &mut ea,
        is_state(dialer.node_id(), ConnectionState::Connected),
    )
    .await;
    wait_for(
        &mut ed,
        is_state(acceptor.node_id(), ConnectionState::Connected),
    )
    .await;
}

#[test]
fn handler_sees_channel_after_hello_on_both_sides() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        let (ha, hb) = (
            RecordingChannelHandler::new(),
            RecordingChannelHandler::new(),
        );
        a.node.set_channel_handler(ha.clone());
        b.node.set_channel_handler(hb.clone());

        connect(&b.node, &a.node).await;

        let on_a = ha.wait_for(&b.id(), WAIT).await.expect("A channel");
        let on_b = hb.wait_for(&a.id(), WAIT).await.expect("B channel");
        assert_eq!(ha.seen().len(), 1);
        assert_eq!(hb.seen().len(), 1);
        assert_eq!(on_a.peer(), b.id());
        assert_eq!(on_b.peer(), a.id());
        assert!(on_a.is_current());
        assert!(on_b.is_current());
        assert_eq!(
            a.node.channel(&b.id()).unwrap().stable_id(),
            on_a.stable_id()
        );
    });
}

#[test]
fn pending_and_declined_peers_never_reach_the_handler() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        let handler = RecordingChannelHandler::new();
        a.node.set_channel_handler(handler.clone());
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node
            .connect_ticket(&a.node.ticket().unwrap().to_string())
            .unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert!(handler.wait_for(&b.id(), QUIET).await.is_none());
        assert!(handler.seen().is_empty());
        assert!(a.node.channel(&b.id()).is_none());

        a.node.respond_incoming(&b.id(), false).unwrap();
        wait_for(&mut eb, |ev| {
            matches!(
                ev,
                NetEvent::ConnectionState {
                    state: ConnectionState::Failed(_),
                    ..
                }
            )
        })
        .await;
        assert!(handler.wait_for(&b.id(), QUIET).await.is_none());
        assert!(handler.seen().is_empty());
    });
}

#[test]
fn handler_registered_late_replays_current_channels() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        connect(&b.node, &a.node).await;

        let handler = RecordingChannelHandler::new();
        a.node.set_channel_handler(handler.clone());

        assert_eq!(handler.seen().len(), 1);
        let channel = handler.wait_for(&b.id(), WAIT).await.unwrap();
        assert!(channel.is_current());
    });
}

#[test]
fn handler_can_query_node_inside_on_channel() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        let b_id = b.id();
        let a_node = Arc::new(a.node);

        // Replay path: B is connected before the handler is registered.
        connect(&b.node, &a_node).await;
        let replay = RecordingChannelHandler::querying(&a_node);
        tokio::time::timeout(WAIT, async { a_node.set_channel_handler(replay.clone()) })
            .await
            .expect("replay completes");
        assert_eq!(replay.queried(), vec![true]);

        // Live path: a second peer authorizes while the handler is set.
        let c = node("Carol").await;
        connect(&c.node, &a_node).await;
        let c_channel = replay.wait_for(&c.id(), WAIT).await.expect("live channel");
        assert!(c_channel.is_current());
        assert_eq!(replay.queried(), vec![true, true]);
        assert!(replay.seen().iter().any(|(peer, _)| *peer == b_id));
    });
}

#[test]
fn replaced_or_closed_channel_is_not_current() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        let handler = RecordingChannelHandler::new();
        a.node.set_channel_handler(handler.clone());
        let mut ea = a.node.subscribe();

        connect(&b.node, &a.node).await;
        let first = handler.wait_for(&b.id(), WAIT).await.unwrap();
        assert!(first.is_current());

        assert!(a.node.disconnect(&b.id()));
        wait_for(&mut ea, is_state(b.id(), ConnectionState::Disconnected)).await;
        assert!(!first.is_current());
        assert!(a.node.channel(&b.id()).is_none());

        connect(&b.node, &a.node).await;
        let seen = handler.seen();
        assert_eq!(seen.len(), 2);
        let second = a.node.channel(&b.id()).unwrap();
        assert_ne!(second.stable_id(), first.stable_id());
        assert_eq!(seen[1].1, second.stable_id());
        assert!(second.is_current());
        assert!(!first.is_current());
    });
}

#[test]
fn authorized_peers_exchange_post_hello_streams() {
    run(async {
        let (a, b) = (node("Alice").await, node("Bob").await);
        let (ha, hb) = (
            RecordingChannelHandler::new(),
            RecordingChannelHandler::new(),
        );
        a.node.set_channel_handler(ha.clone());
        b.node.set_channel_handler(hb.clone());
        connect(&b.node, &a.node).await;
        let on_a = ha.wait_for(&b.id(), WAIT).await.unwrap();
        let on_b = hb.wait_for(&a.id(), WAIT).await.unwrap();

        let (mut send, mut recv) = on_a.open_stream().await.unwrap();
        send.write_all(b"ping").await.unwrap();
        send.finish().unwrap();

        let (mut reply, mut incoming) = on_b.accept_stream().await.unwrap();
        assert_eq!(incoming.read_to_end(64).await.unwrap(), b"ping");
        reply.write_all(b"pong").await.unwrap();
        reply.finish().unwrap();

        assert_eq!(recv.read_to_end(64).await.unwrap(), b"pong");
    });
}

#[test]
fn close_protocol_closes_with_protocol_code() {
    run(async {
        let a = node("Alice").await;
        let handler = RecordingChannelHandler::new();
        a.node.set_channel_handler(handler.clone());
        let mut ea = a.node.subscribe();

        let addr = Ticket::parse(&a.node.ticket().unwrap().to_string())
            .unwrap()
            .addr;
        let dial = raw_dial(addr, &hello_frame("Raw")).await.unwrap();
        let NetEvent::IncomingRequest(req) = wait_for(&mut ea, is_incoming).await else {
            unreachable!()
        };
        a.node.respond_incoming(&req.node_id, true).unwrap();

        let channel = handler.wait_for(&req.node_id, WAIT).await.unwrap();
        channel.close_protocol();

        match tokio::time::timeout(WAIT, dial.conn.closed())
            .await
            .unwrap()
        {
            ConnectionError::ApplicationClosed(close) => {
                assert_eq!(u64::from(close.error_code), u64::from(CLOSE_PROTOCOL));
            }
            other => panic!("unexpected close: {other:?}"),
        }
        assert!(!channel.is_current());
    });
}
