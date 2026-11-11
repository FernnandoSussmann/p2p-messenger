//! Advisory peer names: Hello names on `Connected` events, observations on
//! the trust seam, and the node's small query/disconnect API. The package name
//! `core` shadows std `core`, which breaks `#[tokio::test]`, so tests run
//! through `run`.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use core::discovery::testing::{
    hello_frame, next_event, raw_dial, restart, spawn_test_node_with, KnownFactory,
    MemoryKnownPeers, MemoryLanBus, TestNode, TestOptions,
};
use core::discovery::{
    ConnectionState, Direction, KnownPeers, KnownPeersError, NameSource, NetEvent,
};
use core::identity::{AccountId, DisplayName, IdentityStatus, NodeId, Ticket};
use tokio::sync::broadcast::Receiver;

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(600);

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

fn is_incoming(ev: &NetEvent) -> bool {
    matches!(ev, NetEvent::IncomingRequest(_))
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

fn connected_name(ev: NetEvent) -> Option<String> {
    match ev {
        NetEvent::ConnectionState { display_name, .. } => {
            display_name.map(|n| n.as_str().to_owned())
        }
        other => panic!("not a connection state: {other:?}"),
    }
}

async fn named_node(name: &str, known: Arc<dyn KnownPeers>) -> TestNode {
    spawn_test_node_with(name, TestOptions::default().with_known(known)).await
}

async fn until<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn some(name: &str) -> Option<String> {
    Some(name.to_owned())
}

/// Records every `observe` call, trusting nobody.
#[derive(Default)]
struct SpyKnown {
    observed: Mutex<Vec<NodeId>>,
}

impl KnownPeers for SpyKnown {
    fn is_known(&self, _id: &NodeId) -> Result<bool, KnownPeersError> {
        Ok(false)
    }

    fn add(&self, _id: NodeId, _name: Option<DisplayName>) -> Result<(), KnownPeersError> {
        Ok(())
    }

    fn observe(
        &self,
        id: &NodeId,
        _name: Option<&DisplayName>,
        _source: NameSource,
    ) -> Result<(), KnownPeersError> {
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*id);
        Ok(())
    }
}

/// Connects `dialer` to `target` through the prompt, accepting it.
async fn connect_accepted(dialer: &TestNode, target: &TestNode) {
    let mut et = target.node.subscribe();
    dialer.node.connect_ticket(&ticket(target)).unwrap();
    wait_for(&mut et, is_incoming).await;
    target.node.respond_incoming(&dialer.id(), true).unwrap();
}

#[test]
fn connected_event_carries_hello_name_both_directions() {
    run(async {
        let a = named_node("Alice", Arc::new(MemoryKnownPeers::new())).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        connect_accepted(&b, &a).await;
        let inbound = wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        let outbound = wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(connected_name(inbound), some("Bob"));
        assert_eq!(connected_name(outbound), some("Alice"));
    });
}

#[test]
fn connected_event_name_is_none_for_invalid_hello_name() {
    run(async {
        let a = named_node("Alice", Arc::new(MemoryKnownPeers::new())).await;
        let mut ea = a.node.subscribe();
        let addr = Ticket::parse(&ticket(&a)).unwrap().addr;

        let dial = raw_dial(addr, &hello_frame("Mal\u{202E}ory"))
            .await
            .unwrap();
        let NetEvent::IncomingRequest(req) = wait_for(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert!(req.display_name.is_none());
        a.node.respond_incoming(&req.node_id, true).unwrap();
        let connected = wait_for(
            &mut ea,
            state_is(req.node_id, Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(connected_name(connected), None);
        drop(dial);
    });
}

#[test]
fn known_peer_name_is_observed_on_every_authorized_connection() {
    run(async {
        let known_a = Arc::new(MemoryKnownPeers::new());
        let a = named_node("Alice", known_a.clone()).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        known_a.trust(b.id());
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        until(|| (known_a.observed(&b.id()).len() == 1).then_some(())).await;

        assert!(b.node.disconnect(&a.id()));
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Disconnected),
        )
        .await;
        b.identity.set_display_name("Robert").unwrap();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        until(|| (known_a.observed(&b.id()).len() == 2).then_some(())).await;

        assert_eq!(
            known_a.observed(&b.id()),
            [
                (some("Bob"), NameSource::Hello),
                (some("Robert"), NameSource::Hello)
            ]
        );
        assert_eq!(known_a.len(), 1);
    });
}

#[test]
fn lan_names_are_observed_once_per_change() {
    run(async {
        let bus = MemoryLanBus::new();
        let known_a = Arc::new(MemoryKnownPeers::new());
        let b = spawn_test_node_with("Bob", TestOptions::default().with_lan(&bus)).await;
        known_a.trust(b.id());
        let a = spawn_test_node_with(
            "Alice",
            TestOptions::default()
                .with_lan(&bus)
                .with_known(known_a.clone()),
        )
        .await;
        until(|| (known_a.observed(&b.id()).len() == 1).then_some(())).await;

        b.node.refresh_local_name();
        tokio::time::sleep(QUIET).await;
        b.identity.set_display_name("Robert").unwrap();
        b.node.refresh_local_name();
        until(|| (known_a.observed(&b.id()).len() == 2).then_some(())).await;
        b.node.refresh_local_name();
        tokio::time::sleep(QUIET).await;

        assert_eq!(
            known_a.observed(&b.id()),
            [
                (some("Bob"), NameSource::Lan),
                (some("Robert"), NameSource::Lan)
            ]
        );
        drop(a);
    });
}

#[test]
fn observe_is_never_called_for_declined_or_pending_peers() {
    run(async {
        let spy = Arc::new(SpyKnown::default());
        let a = named_node("Alice", spy.clone()).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        let mut ea = a.node.subscribe();

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        tokio::time::sleep(QUIET).await;
        assert!(spy.observed.lock().unwrap().is_empty(), "pending");

        a.node.respond_incoming(&b.id(), false).unwrap();
        wait_for(&mut ea, |ev| {
            matches!(
                ev,
                NetEvent::ConnectionState {
                    state: ConnectionState::Failed(_),
                    ..
                }
            )
        })
        .await;
        tokio::time::sleep(QUIET).await;
        assert!(spy.observed.lock().unwrap().is_empty(), "declined");
    });
}

#[test]
fn pending_request_returns_open_prompt_only() {
    run(async {
        let a = named_node("Alice", Arc::new(MemoryKnownPeers::new())).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        let mut ea = a.node.subscribe();
        assert!(a.node.pending_request(&b.id()).is_none());

        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        let req = a.node.pending_request(&b.id()).expect("open prompt");
        assert_eq!(req.node_id, b.id());
        assert_eq!(req.display_name.as_ref().map(|n| n.as_str()), Some("Bob"));
        assert!(a.node.pending_request(&a.id()).is_none());

        a.node.respond_incoming(&b.id(), true).unwrap();
        assert!(a.node.pending_request(&b.id()).is_none());
    });
}

#[test]
fn last_seen_name_prefers_authorized_then_pending_then_lan() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = spawn_test_node_with("Alice", TestOptions::default().with_lan(&bus)).await;
        let b = spawn_test_node_with("Bob", TestOptions::default().with_lan(&bus)).await;
        let name = |n: &TestNode, of: &TestNode| {
            n.node
                .last_seen_name(&of.id())
                .map(|d| d.as_str().to_owned())
        };
        until(|| (name(&a, &b).as_deref() == Some("Bob")).then_some(())).await;

        // The LAN record keeps "Bob"; the Hello now says "Robert".
        b.identity.set_display_name("Robert").unwrap();
        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert_eq!(name(&a, &b), some("Robert"), "pending beats LAN");

        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(name(&a, &b), some("Robert"), "authorized");

        assert!(a.node.disconnect(&b.id()));
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Disconnected),
        )
        .await;
        assert_eq!(name(&a, &b), some("Bob"), "falls back to LAN");
        assert_eq!(name(&a, &a), None);
    });
}

#[test]
fn disconnect_closes_authorized_connection_and_emits_disconnected() {
    run(async {
        let a = named_node("Alice", Arc::new(MemoryKnownPeers::new())).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        connect_accepted(&b, &a).await;
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

        assert!(a.node.disconnect(&b.id()));
        wait_for(
            &mut ea,
            state_is(b.id(), Direction::Inbound, ConnectionState::Disconnected),
        )
        .await;
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Disconnected),
        )
        .await;
        assert!(a.node.connected_peers().is_empty());
        assert!(!a.node.disconnect(&b.id()));
    });
}

#[test]
fn disconnect_unknown_peer_is_false_and_harmless() {
    run(async {
        let a = named_node("Alice", Arc::new(MemoryKnownPeers::new())).await;
        let b = named_node("Bob", Arc::new(MemoryKnownPeers::new())).await;
        let mut ea = a.node.subscribe();

        assert!(!a.node.disconnect(&b.id()));
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert!(!a.node.disconnect(&b.id()), "pending is not connected");
        assert!(a.node.pending_request(&b.id()).is_some());
    });
}

#[test]
fn known_factory_receives_account_and_node_id_after_provisioning() {
    run(async {
        let calls: Arc<Mutex<Vec<(AccountId, NodeId)>>> = Arc::default();
        let shared = Arc::new(MemoryKnownPeers::new());
        let factory: KnownFactory = {
            let (calls, shared) = (calls.clone(), shared.clone());
            Arc::new(move |account, node_id| {
                calls.lock().unwrap().push((account, node_id));
                shared.clone() as Arc<dyn KnownPeers>
            })
        };
        let a =
            spawn_test_node_with("Alice", TestOptions::default().with_known_factory(factory)).await;
        let IdentityStatus::Present {
            account_id,
            node_id,
        } = a.identity.status()
        else {
            panic!("provisioned")
        };
        assert_eq!(*calls.lock().unwrap(), [(account_id, node_id)]);

        let a = restart(a).await;
        assert_eq!(a.id(), node_id);
        assert_eq!(
            *calls.lock().unwrap(),
            [(account_id, node_id), (account_id, node_id)]
        );
    });
}
