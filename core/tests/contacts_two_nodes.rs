//! Level-1 contacts integration: real iroh endpoints on loopback whose trust
//! store is a `ContactBook` over a `FileContactStore`. The package name `core`
//! shadows std `core`, which breaks `#[tokio::test]`, so tests run through
//! `run`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use core::contacts::testing::BookCell;
use core::contacts::{
    accept_incoming, migrate_legacy_known_peers, AcceptError, ContactBook, FileContactStore,
    HistoryChoice, NoMessageStore, TrustState,
};
use core::discovery::testing::{
    hello_frame, next_event, raw_dial, restart, spawn_test_node_with, MemoryLanBus, TestNode,
    TestOptions,
};
use core::discovery::{ConnectionState, Direction, NetEvent};
use core::identity::{AccountId, DisplayNameError, IdentityStatus, NodeId, Ticket};
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

fn options(dir: &Path, cell: &BookCell) -> TestOptions {
    let factory = cell.known_factory(
        Arc::new(FileContactStore::new(dir)),
        Arc::new(NoMessageStore),
    );
    TestOptions::default().with_known_factory(factory)
}

async fn cnode(name: &str, dir: &Path) -> (TestNode, BookCell) {
    let cell = BookCell::new();
    (spawn_test_node_with(name, options(dir, &cell)).await, cell)
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

fn self_name_of(book: &ContactBook, id: &NodeId) -> Option<String> {
    book.get(id)
        .unwrap()
        .and_then(|c| c.self_name.map(|n| n.as_str().to_owned()))
}

/// `from` dials `to`, `to` prompts and accepts with `nickname`; both connect.
async fn dial_and_accept(from: &TestNode, to: &TestNode, to_cell: &BookCell, nickname: &str) {
    let (mut efrom, mut eto) = (from.node.subscribe(), to.node.subscribe());
    from.node.connect_ticket(&ticket(to)).unwrap();
    wait_for(&mut eto, is_incoming).await;
    accept_incoming(&to_cell.get(), &to.node, &from.id(), nickname).unwrap();
    wait_for(
        &mut efrom,
        state_is(to.id(), Direction::Outbound, ConnectionState::Connected),
    )
    .await;
    wait_for(
        &mut eto,
        state_is(from.id(), Direction::Inbound, ConnectionState::Connected),
    )
    .await;
}

async fn drop_connection(from: &TestNode, to: &TestNode) {
    let mut eto = to.node.subscribe();
    assert!(from.node.disconnect(&to.id()));
    wait_for(
        &mut eto,
        state_is(from.id(), Direction::Inbound, ConnectionState::Disconnected),
    )
    .await;
}

#[test]
fn accept_with_nickname_lists_unverified_contact_and_reconnects_silently() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        dial_and_accept(&b, &a, &cell_a, "Bobby").await;

        let list = cell_a.get().list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].node_id, b.id());
        assert_eq!(list[0].nickname.as_str(), "Bobby");
        assert_eq!(list[0].self_name.as_ref().unwrap().as_str(), "Bob");
        assert_eq!(list[0].trust, TrustState::Unverified);

        let mut eb = b.node.subscribe();
        let a_id = a.id();
        let a = restart(a).await;
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
    });
}

#[test]
fn dialed_peer_becomes_unverified_contact_with_default_nickname() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, cell_b) = cnode("Bob", dir_b.path()).await;
        dial_and_accept(&b, &a, &cell_a, "Bobby").await;

        let list = cell_b.get().list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].node_id, a.id());
        assert_eq!(list[0].nickname.as_str(), "Alice");
        assert_eq!(list[0].trust, TrustState::Unverified);

        let b_id = b.id();
        let mut ea = a.node.subscribe();
        let b = restart(b).await;
        wait_for(
            &mut ea,
            state_is(b_id, Direction::Inbound, ConnectionState::Disconnected),
        )
        .await;
        let mut eb = b.node.subscribe();
        a.node.connect_ticket(&ticket(&b)).unwrap();
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(count_incoming_for(&mut eb, QUIET).await, 0);
    });
}

#[test]
fn peer_rename_updates_self_name_not_nickname() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, cell_b) = cnode("Bob", dir_b.path()).await;
        dial_and_accept(&b, &a, &cell_a, "Bobby").await;

        drop_connection(&b, &a).await;
        b.identity.set_display_name("Robert").unwrap();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        until(|| (self_name_of(&cell_a.get(), &b.id()).as_deref() == Some("Robert")).then_some(()))
            .await;
        let c = cell_a.get().get(&b.id()).unwrap().unwrap();
        assert_eq!(c.nickname.as_str(), "Bobby");

        drop_connection(&b, &a).await;
        a.identity.set_display_name("Alicia").unwrap();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        until(|| (self_name_of(&cell_b.get(), &a.id()).as_deref() == Some("Alicia")).then_some(()))
            .await;
        let c = cell_b.get().get(&a.id()).unwrap().unwrap();
        assert_eq!(c.nickname.as_str(), "Alice");
    });
}

#[test]
fn lan_name_change_updates_self_name_of_existing_contact_only() {
    run(async {
        let bus = MemoryLanBus::new();
        let dir_a = tempfile::tempdir().unwrap();
        let cell = BookCell::new();
        let a = spawn_test_node_with("Alice", options(dir_a.path(), &cell).with_lan(&bus)).await;
        let lan = |name: &'static str| {
            let bus = bus.clone();
            async move { spawn_test_node_with(name, TestOptions::default().with_lan(&bus)).await }
        };
        let b = lan("Bob").await;
        let c = lan("Carol").await;
        let book = cell.get();
        book.add(b.id(), "Bobby", None).unwrap();

        b.identity.set_display_name("Robert").unwrap();
        b.node.refresh_local_name();
        until(|| (self_name_of(&book, &b.id()).as_deref() == Some("Robert")).then_some(())).await;
        c.identity.set_display_name("Caroline").unwrap();
        c.node.refresh_local_name();
        tokio::time::sleep(QUIET).await;

        let list = book.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].nickname.as_str(), "Bobby");
        assert!(!book.is_known(&c.id()).unwrap());
        drop(a);
    });
}

#[test]
fn removed_contact_is_prompted_again() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        dial_and_accept(&b, &a, &cell_a, "Bobby").await;
        drop_connection(&b, &a).await;

        cell_a.get().remove(&b.id(), HistoryChoice::Keep).unwrap();
        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0);
        assert_eq!(a.node.pending_incoming().len(), 1);
    });
}

#[test]
fn decline_creates_no_contact() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        a.node.respond_incoming(&b.id(), false).unwrap();
        tokio::time::sleep(QUIET).await;
        assert!(cell_a.get().list().unwrap().is_empty());
        assert!(!dir_a.path().join("contacts.json").exists());
    });
}

#[test]
fn accept_with_invalid_nickname_keeps_prompt_open() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;

        let book = cell_a.get();
        let bidi = accept_incoming(&book, &a.node, &b.id(), "\u{202E}x");
        assert!(matches!(
            bidi,
            Err(AcceptError::InvalidNickname(DisplayNameError::BidiOverride))
        ));
        let invisible = accept_incoming(&book, &a.node, &b.id(), "a\u{200B}b");
        assert!(matches!(
            invisible,
            Err(AcceptError::InvalidNickname(
                DisplayNameError::InvisibleCharacter
            ))
        ));
        assert_eq!(a.node.pending_incoming().len(), 1);
        assert!(book.list().unwrap().is_empty());
        assert!(a.node.connected_peers().is_empty());
    });
}

#[test]
fn hello_name_with_invisible_char_is_dropped() {
    run(async {
        let dir_a = tempfile::tempdir().unwrap();
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let mut ea = a.node.subscribe();
        let addr = Ticket::parse(&ticket(&a)).unwrap().addr;
        let _dial = raw_dial(addr, &hello_frame("Bob\u{200B}")).await.unwrap();
        let NetEvent::IncomingRequest(req) = wait_for(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert!(req.display_name.is_none());

        let c = accept_incoming(&cell_a.get(), &a.node, &req.node_id, "Raw").unwrap();
        assert!(c.self_name.is_none());
        assert!(self_name_of(&cell_a.get(), &req.node_id).is_none());
    });
}

#[test]
fn corrupt_contacts_file_fails_closed_and_prompts() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        dial_and_accept(&b, &a, &cell_a, "Bobby").await;
        drop_connection(&b, &a).await;

        let path = dir_a.path().join("contacts.json");
        std::fs::write(&path, "garbage").unwrap();
        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert_eq!(std::fs::read(&path).unwrap(), b"garbage");
    });
}

#[test]
fn legacy_known_peers_reconnect_silently_after_migration() {
    run(async {
        let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, cell_a) = cnode("Alice", dir_a.path()).await;
        let (b, _) = cnode("Bob", dir_b.path()).await;
        let IdentityStatus::Present { account_id, .. } = a.identity.status() else {
            panic!("provisioned")
        };
        let legacy = serde_json::json!({"version": 1, "accounts": {
            account_id.as_uuid().hyphenated().to_string():
                {b.id().to_string(): {"name": "Bob", "added_at_ms": 7}}
        }});
        let legacy_path = dir_a.path().join("known_peers.json");
        std::fs::write(&legacy_path, legacy.to_string()).unwrap();
        let store = FileContactStore::new(dir_a.path());
        let report = migrate_legacy_known_peers(dir_a.path(), &store).unwrap();
        assert_eq!(report.migrated, 1);
        assert!(!legacy_path.exists());

        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(
            &mut eb,
            state_is(a.id(), Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0);
        let c = cell_a.get().get(&b.id()).unwrap().unwrap();
        assert_eq!(c.nickname.as_str(), "Bob");
        assert_eq!(c.trust, TrustState::Unverified);
    });
}

#[test]
fn contacts_are_namespaced_by_account_end_to_end() {
    run(async {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FileContactStore::new(dir.path()));
        let cell = BookCell::new();
        let factory = cell.known_factory(store.clone(), Arc::new(NoMessageStore));
        let a =
            spawn_test_node_with("Alice", TestOptions::default().with_known_factory(factory)).await;
        let (b, _) = cnode("Bob", tempfile::tempdir().unwrap().path()).await;

        let other = ContactBook::new(AccountId::new(), a.id(), store, Arc::new(NoMessageStore));
        other.add(b.id(), "Bobby", None).unwrap();
        assert!(other.is_known(&b.id()).unwrap());
        assert!(!cell.get().is_known(&b.id()).unwrap());

        let mut ea = a.node.subscribe();
        b.node.connect_ticket(&ticket(&a)).unwrap();
        wait_for(&mut ea, is_incoming).await;
    });
}
