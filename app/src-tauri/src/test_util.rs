//! Shared test helpers: checks that serialized DTOs never embed raw key bytes,
//! and the two-node network fixtures used by the command tests.

use std::sync::Arc;
use std::time::Duration;

use messenger_core::contacts::testing::{MemoryContactStore, RecordingHistoryEraser};
use messenger_core::discovery::testing::{
    next_event, spawn_test_node_with, TestIdentity, TestOptions,
};
use messenger_core::discovery::{ConnectionState, Direction, KnownPeers, NetEvent};
use messenger_core::identity::testing::FakeKeychain;
use messenger_core::identity::NodeId;
use tokio::sync::broadcast::Receiver;

use crate::contacts_cmd::ContactsState;
use crate::net_cmd::{my_ticket, NetState};

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
