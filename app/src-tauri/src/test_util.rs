//! Shared test helpers: checks that serialized DTOs never embed raw key bytes,
//! and the two-node network and chat fixtures used by the command tests.

use std::sync::Arc;
use std::time::Duration;

use messenger_core::contacts::testing::{MemoryContactStore, RecordingHistoryEraser};
use messenger_core::discovery::testing::{
    next_event, spawn_test_node_with, TestIdentity, TestOptions,
};
use messenger_core::discovery::{ConnectionState, Direction, KnownPeers, NetEvent};
use messenger_core::identity::testing::FakeKeychain;
use messenger_core::identity::{read_marker, NodeId};
use messenger_core::messaging::testing::Faults;
use messenger_core::messaging::{Messaging, MessagingConfig};
use messenger_core::storage::Storage;
use serde_json::Value;
use tokio::sync::broadcast::Receiver;

use crate::chat_cmd::{forward_chat, ChatState};
use crate::contacts_cmd::ContactsState;
use crate::net_cmd::{my_ticket, respond_incoming, NetCmdError, NetState};

pub const WAIT: Duration = Duration::from_secs(10);

/// A started state plus the fixtures that must outlive it.
pub struct Started {
    pub state: NetState,
    pub contacts: Arc<ContactsState>,
    pub history: Arc<RecordingHistoryEraser>,
    pub id: NodeId,
    pub identity: TestIdentity,
    pub keychain: Arc<FakeKeychain>,
    _dir: tempfile::TempDir,
}

pub async fn started_with(name: &str, options: TestOptions) -> Started {
    let history = Arc::new(RecordingHistoryEraser::new());
    let contacts = Arc::new(ContactsState::new(
        Arc::new(MemoryContactStore::new()),
        history.clone(),
    ));
    let bound = contacts.clone();
    let options = options.with_known_factory(Arc::new(move |account, own| {
        bound.bind(account, own) as Arc<dyn KnownPeers>
    }));
    let rest = spawn_test_node_with(name, options).await;
    let id = rest.id();
    let state = NetState::default();
    let node = rest.node;
    assert!(state.ensure_started_with(async { Ok(node) }).await.unwrap());
    Started {
        state,
        contacts,
        history,
        id,
        identity: rest.identity,
        keychain: rest.keychain,
        _dir: rest.dir,
    }
}

pub async fn started(name: &str) -> Started {
    started_with(name, TestOptions::default()).await
}

/// The network and contacts halves of a fixture node.
pub trait Fixture {
    fn net(&self) -> &NetState;
    fn contacts(&self) -> &ContactsState;
}

impl Fixture for Started {
    fn net(&self) -> &NetState {
        &self.state
    }
    fn contacts(&self) -> &ContactsState {
        &self.contacts
    }
}

/// A started node with the real message store, the real history eraser and a
/// running messaging engine.
pub struct StartedChat {
    pub state: NetState,
    pub contacts: Arc<ContactsState>,
    pub chat: Arc<ChatState>,
    pub messaging: Arc<Messaging>,
    pub faults: Arc<Faults>,
    pub id: NodeId,
    pub identity: TestIdentity,
    pub keychain: Arc<FakeKeychain>,
    _dir: tempfile::TempDir,
}

impl Fixture for StartedChat {
    fn net(&self) -> &NetState {
        &self.state
    }
    fn contacts(&self) -> &ContactsState {
        &self.contacts
    }
}

pub async fn started_with_chat(name: &str) -> StartedChat {
    let chat = Arc::new(ChatState::new());
    let contacts = Arc::new(ContactsState::new(
        Arc::new(MemoryContactStore::new()),
        chat.clone(),
    ));
    let bound = contacts.clone();
    let options = TestOptions::default().with_known_factory(Arc::new(move |account, own| {
        bound.bind(account, own) as Arc<dyn KnownPeers>
    }));
    let rest = spawn_test_node_with(name, options).await;
    let id = rest.id();
    let state = NetState::default();
    let node = rest.node;
    assert!(state.ensure_started_with(async { Ok(node) }).await.unwrap());
    let account = read_marker(rest.dir.path()).unwrap().expect("provisioned");
    let storage = Storage::new(rest.dir.path(), rest.keychain.clone());
    storage.database_creating(&account).expect("create store");
    chat.open_store(&storage, &account).expect("open store");
    let config = MessagingConfig::fast();
    let faults = config.faults.clone();
    let book = contacts.book().expect("contacts are bound");
    let messaging = chat
        .start_messaging(&state.node().unwrap(), book, config)
        .await
        .expect("start messaging");
    StartedChat {
        state,
        contacts,
        chat,
        messaging,
        faults,
        id,
        identity: rest.identity,
        keychain: rest.keychain,
        _dir: rest.dir,
    }
}

pub async fn accept(s: &impl Fixture, peer: &NodeId, nickname: &str) -> Result<(), NetCmdError> {
    respond_incoming(
        s.net(),
        s.contacts(),
        &peer.to_string(),
        true,
        Some(nickname.to_owned()),
    )
    .await
}

pub async fn decline(s: &impl Fixture, peer: &NodeId) -> Result<(), NetCmdError> {
    respond_incoming(s.net(), s.contacts(), &peer.to_string(), false, None).await
}

fn connected_to(peer: NodeId) -> impl Fn(&NetEvent) -> bool {
    move |ev| {
        matches!(ev, NetEvent::ConnectionState { node_id, state: ConnectionState::Connected, .. }
            if *node_id == peer)
    }
}

/// `a` dials `b` by ticket. `b` accepts a prompt if one arrives (a known
/// contact connects without one). Returns once both sides are connected and
/// both engines hold the channel.
pub async fn connect_pair(a: &StartedChat, b: &StartedChat) {
    let (mut ea, mut eb) = (
        a.state.node().unwrap().subscribe(),
        b.state.node().unwrap().subscribe(),
    );
    let ticket = my_ticket(&b.state).await.unwrap().ticket;
    a.state.node().unwrap().connect_ticket(&ticket).unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(&mut eb, left).await {
            Some(NetEvent::IncomingRequest(_)) => accept(b, &a.id, "peer").await.unwrap(),
            Some(ev) if connected_to(a.id)(&ev) => break,
            Some(_) => {}
            None => panic!("b never connected"),
        }
    }
    expect(&mut ea, connected_to(b.id)).await;
    while !(a.messaging.has_channel(&b.id) && b.messaging.has_channel(&a.id)) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "engines never saw the channel"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Chat events as the webview would receive them, via `forward_chat`.
#[derive(Clone, Default)]
pub struct ChatLog(Arc<std::sync::Mutex<Vec<(String, Value)>>>);

impl ChatLog {
    pub fn record(messaging: &Messaging) -> Self {
        let log = Self::default();
        let (sink, mut rx) = (log.0.clone(), messaging.subscribe());
        tokio::spawn(async move {
            while forward_chat(rx.recv().await, |name, payload| {
                sink.lock().unwrap().push((name.to_owned(), payload));
            }) {}
        });
        log
    }

    pub fn all(&self) -> Vec<(String, Value)> {
        self.0.lock().unwrap().clone()
    }

    pub fn named(&self, name: &str) -> Vec<Value> {
        self.all()
            .into_iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }

    pub async fn wait_status(&self, id: &str, status: &str) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !self
            .named("message_status")
            .iter()
            .any(|v| v["id"] == id && v["status"] == status)
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "status {status} never arrived"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

pub fn events(s: &Started) -> Receiver<NetEvent> {
    s.state.node().unwrap().subscribe()
}

pub async fn expect(rx: &mut Receiver<NetEvent>, pred: impl Fn(&NetEvent) -> bool) -> NetEvent {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(rx, left).await {
            Some(ev) if pred(&ev) => return ev,
            Some(_) => {}
            None => panic!("timed out waiting for event"),
        }
    }
}

pub fn is_state(
    peer: NodeId,
    direction: Direction,
    state: ConnectionState,
) -> impl Fn(&NetEvent) -> bool {
    move |ev| {
        matches!(ev, NetEvent::ConnectionState { node_id, direction: d, state: s, .. }
            if *node_id == peer && *d == direction && *s == state)
    }
}

pub fn is_incoming(ev: &NetEvent) -> bool {
    matches!(ev, NetEvent::IncomingRequest(_))
}

pub async fn ticket_of(s: &Started) -> String {
    my_ticket(&s.state).await.unwrap().ticket
}

pub fn assert_forms_absent(json: &str, key: &[u8; 32]) {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let decimal = format!("{key:?}");
    let decimal_compact = decimal.replace(' ', "");
    assert!(!json.to_lowercase().contains(&hex));
    assert!(!json.contains(&decimal));
    assert!(!json.contains(&decimal_compact));
    assert!(!json.contains(&base64_standard(key)));
}

pub fn base64_standard(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
