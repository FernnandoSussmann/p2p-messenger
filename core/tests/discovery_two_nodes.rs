//! Level-1 harness: real iroh endpoints on loopback, in one process, driving
//! the same `Node` code as production. The package name `core` shadows std
//! `core`, which breaks `#[tokio::test]`, so tests run through `run`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use core::contacts::testing::BookCell;
use core::contacts::{accept_incoming, FileContactStore, NoMessageStore, TrustState};
use core::discovery::testing::{
    hello_frame, next_event, raw_dial, restart, spawn_test_node_with, FailingKnownPeers,
    MemoryAddressLookup, MemoryKnownPeers, TestNode, TestOptions,
};
use core::discovery::{
    event_json, ConnectError, ConnectionState, Direction, FailureReason, GateLimits, MyTicketDto,
    NetEvent, CLOSE_PROTOCOL,
};
use core::identity::{KeychainStore, NodeId, Ticket};
use iroh::endpoint::ConnectionError;
use protocol::{new_message_id, ChatMessage, Envelope, Payload};
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

async fn node(name: &str) -> (TestNode, Arc<MemoryKnownPeers>) {
    let known = Arc::new(MemoryKnownPeers::new());
    let options = TestOptions::default().with_known(known.clone());
    (spawn_test_node_with(name, options).await, known)
}

/// A node whose trust store is a `ContactBook` over a file store in `dir`.
async fn contacts_node(name: &str, dir: &Path) -> (TestNode, BookCell) {
    let cell = BookCell::new();
    let factory = cell.known_factory(
        Arc::new(FileContactStore::new(dir)),
        Arc::new(NoMessageStore),
    );
    let options = TestOptions::default().with_known_factory(factory);
    (spawn_test_node_with(name, options).await, cell)
}

fn accept(node: &TestNode, cell: &BookCell, peer: &NodeId, nickname: &str) {
    accept_incoming(&cell.get(), &node.node, peer, nickname).unwrap();
}

fn ticket(node: &TestNode) -> String {
    node.node.ticket().unwrap().to_string()
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

fn state_is(
    peer: NodeId,
    direction: Direction,
    state: ConnectionState,
) -> impl Fn(&NetEvent) -> bool {
    move |ev| {
        matches!(ev, NetEvent::ConnectionState { node_id, direction: d, state: s, .. }
            if *node_id == peer && *d == direction && *s == state)
    }
}

fn is_incoming(ev: &NetEvent) -> bool {
    matches!(ev, NetEvent::IncomingRequest(_))
}

async fn count_incoming_for(rx: &mut Receiver<NetEvent>, window: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + window;
    let mut count = 0;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(rx, left).await {
            Some(NetEvent::IncomingRequest(_)) => count += 1,
            Some(_) => {}
            None => return count,
        }
    }
}

async fn no_events(rx: &mut Receiver<NetEvent>) {
    if let Some(ev) = next_event(rx, QUIET).await {
        panic!("unexpected event: {ev:?}");
    }
}

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&path).unwrap_or_default(),
            )
        })
        .collect()
}

fn addr_of(node: &TestNode) -> iroh::EndpointAddr {
    Ticket::parse(&ticket(node)).unwrap().addr
}

#[test]
fn ticket_round_trip_yields_live_channel() {
    run(async {
        let (a, known_a) = node("Alice").await;
        let (b, known_b) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();

        let NetEvent::IncomingRequest(req) = wait_for(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert_eq!(req.node_id, b.id());
        assert_eq!(req.display_name.as_ref().map(|n| n.as_str()), Some("Bob"));

        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0);

        assert_eq!(a.node.connected_peers(), vec![b.id()]);
        assert_eq!(b.node.connected_peers(), vec![a.id()]);
        assert!(a.node.pending_incoming().is_empty());
        // Inbound grants nothing without contacts; the dialer remembers.
        assert!(known_a.is_empty());
        assert_eq!(known_b.dialed(), vec![a.id()]);
    });
}

#[test]
fn corrupted_ticket_is_handled_and_endpoint_stays_up() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let mut ea = a.node.subscribe();
        let good = ticket(&a);

        let bad = [
            good[..good.len() / 2].to_owned(),
            "garbage".to_owned(),
            String::new(),
            "p2pm:".to_owned(),
            "p2pm:zzzz".to_owned(),
            "x".repeat(100_000),
            format!("p2pm:{}", "a".repeat(5_000)),
        ];
        for input in &bad {
            assert_eq!(
                b.node.connect_ticket(input),
                Err(ConnectError::InvalidTicket),
                "{input:.40}"
            );
        }

        b.node.connect_ticket(&good).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
    });
}

#[test]
fn unknown_peer_raises_exactly_one_prompt_and_gets_no_traffic_before_accept() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;

        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0);
        assert!(a.node.connected_peers().is_empty());
        assert!(b.node.connected_peers().is_empty());
        assert_eq!(a.node.pending_incoming().len(), 1);
        // B's Hello-reply read is still pending: nothing but `Connecting` so far.
        while let Some(ev) = next_event(&mut eb, QUIET).await {
            assert!(
                state_is(a.id(), Direction::Outbound, ConnectionState::Connecting)(&ev),
                "{ev:?}"
            );
        }

        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(a.node.connected_peers(), vec![b.id()]);
        assert_eq!(b.node.connected_peers(), vec![a.id()]);
    });
}

#[test]
fn decline_closes_and_persists_nothing() {
    run(async {
        let (a, known_a) = node("Alice").await;
        let (b, known_b) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        let before = snapshot(a.dir.path());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), false).unwrap();

        wait_for(
            &mut eb,
            state_is(
                a.id(),
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        assert!(known_a.is_empty());
        assert!(known_b.is_empty());
        assert!(a.node.connected_peers().is_empty());
        assert_eq!(snapshot(a.dir.path()), before);
    });
}

#[test]
fn pending_prompt_expires() {
    run(async {
        let limits = GateLimits {
            prompt_ttl: Duration::from_millis(300),
            ..GateLimits::default()
        };
        let known_a = Arc::new(MemoryKnownPeers::new());
        let a = spawn_test_node_with(
            "Alice",
            TestOptions::default()
                .with_known(known_a.clone())
                .with_limits(limits),
        )
        .await;
        let (b, known_b) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        let before = snapshot(a.dir.path());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        wait_for(
            &mut ea,
            state_is(
                b.id(),
                Direction::Inbound,
                ConnectionState::Failed(FailureReason::Expired),
            ),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(
                a.id(),
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;

        assert!(a.node.pending_incoming().is_empty());
        assert!(known_a.is_empty() && known_b.is_empty());
        assert_eq!(snapshot(a.dir.path()), before);
        assert_eq!(
            a.node.respond_incoming(&b.id(), true),
            Err(core::discovery::RespondError::NoSuchRequest)
        );
    });
}

#[test]
fn known_peer_reconnects_silently_across_restart() {
    run(async {
        let known_dir = tempfile::tempdir().unwrap();
        let (a, cell) = contacts_node("Alice", known_dir.path()).await;
        let (b, _) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        accept(&a, &cell, &b.id(), "Bobby");
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        let a_id = a.id();
        assert!(known_dir.path().join("contacts.json").is_file());

        let a = restart(a).await;
        assert_eq!(a.id(), a_id);
        wait_for(
            &mut eb,
            state_is(a_id, Direction::Outbound, ConnectionState::Disconnected),
        )
        .await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a_id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0);
        assert!(a.node.pending_incoming().is_empty());
    });
}

#[test]
fn flood_guard_bounds_prompts() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let mut ea = a.node.subscribe();
        let limits = GateLimits::default();
        let ticket_a = ticket(&a);

        for _ in 0..10 {
            b.node.connect_ticket(&ticket_a).unwrap();
        }
        let prompts = count_incoming_for(&mut ea, Duration::from_secs(2)).await;
        assert!(prompts >= 1);
        assert!(
            prompts <= limits.max_attempts_per_peer as usize,
            "{prompts}"
        );
        assert!(a.node.pending_incoming().len() <= 1);
        a.node.respond_incoming(&b.id(), false).unwrap();
        assert!(a.node.pending_incoming().is_empty());

        let addr = addr_of(&a);
        let mut dialers = Vec::new();
        for i in 0..8 {
            dialers.push(
                raw_dial(addr.clone(), &hello_frame(&format!("peer{i}")))
                    .await
                    .unwrap(),
            );
        }
        let prompts = count_incoming_for(&mut ea, Duration::from_secs(2)).await;
        assert_eq!(prompts, limits.max_pending);
        assert_eq!(a.node.pending_incoming().len(), limits.max_pending);

        // The three dropped dialers were closed with no prompt and no event.
        let mut closed = 0;
        for d in &dialers {
            if tokio::time::timeout(Duration::from_millis(100), d.conn.closed())
                .await
                .is_ok()
            {
                closed += 1;
            }
        }
        assert_eq!(closed, 8 - limits.max_pending);
        no_events(&mut ea).await;
    });
}

#[test]
fn non_hello_first_frame_and_oversized_hello_are_rejected() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let mut ea = a.node.subscribe();
        let addr = addr_of(&a);

        let chat = Envelope::new(Payload::Chat(ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1,
            body: "hi".into(),
        }))
        .encode()
        .unwrap();
        let mut random = vec![0u8; 300];
        rand::fill(&mut random[..]);
        let frames = [chat, vec![0x41; 64 * 1024], random];

        for frame in &frames {
            let dial = raw_dial(addr.clone(), frame).await.unwrap();
            let reason = tokio::time::timeout(WAIT, dial.conn.closed())
                .await
                .unwrap();
            match reason {
                ConnectionError::ApplicationClosed(close) => {
                    assert_eq!(u64::from(close.error_code), u64::from(CLOSE_PROTOCOL));
                }
                other => panic!("unexpected close: {other:?}"),
            }
            no_events(&mut ea).await;
            assert!(a.node.pending_incoming().is_empty());
            assert!(a.node.connected_peers().is_empty());
        }

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
    });
}

#[test]
fn self_dial_is_rejected() {
    run(async {
        let (a, _) = node("Alice").await;
        assert_eq!(
            a.node.connect_ticket(&ticket(&a)),
            Err(ConnectError::SelfDial)
        );
        assert_eq!(
            a.node.connect_ticket(&format!("p2pm:{}", a.id())),
            Err(ConnectError::SelfDial)
        );
    });
}

#[test]
fn connection_drop_emits_disconnected() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;

        let b_id = b.id();
        b.node.shutdown().await;
        wait_for(
            &mut ea,
            state_is(b_id, Direction::Inbound, ConnectionState::Disconnected),
        )
        .await;
        assert!(a.node.connected_peers().is_empty());
    });
}

#[test]
fn declined_inbound_event_is_inbound_and_leaves_no_record() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        let before = snapshot(a.dir.path());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), false).unwrap();

        wait_for(
            &mut ea,
            state_is(
                b.id(),
                Direction::Inbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(
                a.id(),
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        assert!(a.node.connected_peers().is_empty());
        assert!(a.node.pending_incoming().is_empty());
        assert_eq!(snapshot(a.dir.path()), before);
    });
}

#[test]
fn cached_node_id_dials_via_address_lookup() {
    run(async {
        let lookup = MemoryAddressLookup::new();
        let options = || TestOptions::default().with_lookup(&lookup);
        let a = spawn_test_node_with("Alice", options()).await;
        let b = spawn_test_node_with("Bob", options()).await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        let published = async {
            loop {
                if lookup
                    .published(&a.id())
                    .is_some_and(|addr| addr.ip_addrs().next().is_some())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(WAIT, published).await.unwrap();

        b.node.connect_ticket(&format!("p2pm:{}", a.id())).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
    });
}

#[test]
fn unresolvable_node_id_fails_as_unreachable() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let mut eb = b.node.subscribe();
        let a_id = a.id();
        a.node.shutdown().await;

        b.node.connect_ticket(&format!("p2pm:{a_id}")).unwrap();
        wait_for(
            &mut eb,
            state_is(
                a_id,
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Unreachable),
            ),
        )
        .await;
        assert!(b.node.connected_peers().is_empty());
    });
}

#[test]
fn storage_error_fails_closed_and_prompts() {
    run(async {
        let a = spawn_test_node_with(
            "Alice",
            TestOptions::default().with_known(Arc::new(FailingKnownPeers)),
        )
        .await;
        let (b, _) = node("Bob").await;
        let mut ea = a.node.subscribe();

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert!(a.node.connected_peers().is_empty());
        assert_eq!(a.node.pending_incoming().len(), 1);
    });
}

#[test]
fn dialed_peer_is_remembered_only_after_a_valid_reply() {
    run(async {
        let (a, _) = node("Alice").await;
        let dir_b = tempfile::tempdir().unwrap();
        let (b, cell_b) = contacts_node("Bob", dir_b.path()).await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert!(cell_b.get().list().unwrap().is_empty());

        a.node.respond_incoming(&b.id(), false).unwrap();
        wait_for(
            &mut eb,
            state_is(
                a.id(),
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        assert!(cell_b.get().list().unwrap().is_empty());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        let contacts = cell_b.get().list().unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].node_id, a.id());
        assert_eq!(contacts[0].nickname.as_str(), "Alice");
        assert_eq!(contacts[0].trust, TrustState::Unverified);
    });
}

#[test]
fn ticket_when_ready_waits_then_returns_and_never_hangs() {
    run(async {
        let (a, _) = node("Alice").await;

        let ticket = a
            .node
            .ticket_when_ready(Duration::from_secs(5))
            .await
            .unwrap();
        let parsed = Ticket::parse(ticket.as_str()).unwrap();
        assert_eq!(parsed.node_id, a.id());
        assert!(parsed.addr.ip_addrs().next().is_some());

        let started = std::time::Instant::now();
        let quick = a
            .node
            .ticket_when_ready(Duration::from_millis(1))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(Ticket::parse(quick.as_str()).is_ok());
    });
}

#[test]
fn my_ticket_changed_fires_when_addresses_change() {
    run(async {
        let (a, _) = node("Alice").await;
        let mut ea = a.node.subscribe();
        // Let start-up address discovery settle before changing anything.
        a.node
            .ticket_when_ready(Duration::from_secs(5))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        while next_event(&mut ea, Duration::from_millis(100))
            .await
            .is_some()
        {}

        a.node
            .add_test_external_addr("192.0.2.1:4000".parse().unwrap())
            .await;
        wait_for(&mut ea, |ev| *ev == NetEvent::MyTicketChanged).await;
        assert!(ticket(&a).contains("p2pm:"));
    });
}

fn base64(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(alphabet[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[test]
fn identity_key_never_leaves_core() {
    run(async {
        let (a, _) = node("Alice").await;
        let (b, _) = node("Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), true).unwrap();

        let mut json = Vec::new();
        for rx in [&mut ea, &mut eb] {
            while let Some(ev) = next_event(rx, QUIET).await {
                json.push(event_json(&ev).1.to_string());
            }
        }
        let info = a.identity.my_info().unwrap();
        json.push(
            serde_json::to_string(&MyTicketDto {
                ticket: info.ticket.to_string(),
                qr_svg: info.qr_svg,
            })
            .unwrap(),
        );
        let all = json.join("\n");
        assert!(all.contains("connection_state") || all.contains("node_id"));

        let account = a.identity.load().unwrap().account_id;
        let secret = *a.keychain.load_identity_key(&account).unwrap().as_bytes();
        let std64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let url64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let canaries = [
            hex,
            format!("{secret:?}"),
            secret
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(","),
            base64(&secret, std64),
            base64(&secret, url64),
        ];
        for canary in canaries {
            assert!(!all.contains(&canary), "key material leaked: {canary:.8}");
        }
    });
}
