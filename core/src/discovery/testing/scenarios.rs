//! Trust-gate and presence scenarios written against [`NodeDriver`], so the
//! same code runs on in-process nodes (Level 1) and on real headless
//! processes (Level 3). They assert on the JSON event payloads only.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::{restart, spawn_test_node_wrapped, SettingsWrapper, TestNode, TestOptions};
use crate::contacts::testing::BookCell;
use crate::contacts::{accept_incoming, Nickname, NoMessageStore, SqlCipherContactStore};
use crate::discovery::event_json;
use crate::identity::{
    AccountId, DbKey, DisplayName, KeychainError, MemorySettingsStore, SettingsError,
    SettingsStore, SqlCipherSettingsStore, Theme,
};
use crate::storage::{KeySource, Storage};

const EVENT_BUFFER: usize = 256;
const POLL: Duration = Duration::from_millis(25);
const STEP: Duration = Duration::from_secs(20);
const QUIET: Duration = Duration::from_millis(600);

/// One node under test, reached through whatever control surface the level
/// provides.
#[allow(async_fn_in_trait)]
pub trait NodeDriver {
    async fn node_id(&self) -> String;
    async fn ticket(&self) -> String;
    async fn connect(&self, ticket: &str) -> Result<String, String>;
    async fn respond(&self, node_id: &str, accept: bool) -> Result<(), String>;
    async fn set_presence(&self, on: bool) -> Result<bool, String>;
    async fn presence(&self) -> bool;
    async fn connected(&self) -> Vec<String>;
    /// Waits for an event by Tauri event name and returns its JSON payload.
    /// `node` and `state` filter on the payload's `node_id` and `state`.
    /// Each event is returned at most once: a later call never sees an event
    /// an earlier call already returned.
    async fn wait_event(
        &self,
        name: &str,
        node: Option<&str>,
        state: Option<&str>,
        timeout: Duration,
    ) -> Option<Value>;
    /// Stops the node and starts it again over the same data and keyring.
    async fn restart(&mut self);
}

async fn expect_event(
    d: &impl NodeDriver,
    name: &str,
    node: &str,
    state: Option<&str>,
    what: &str,
) -> Value {
    d.wait_event(name, Some(node), state, STEP)
        .await
        .unwrap_or_else(|| panic!("timed out waiting for {what}"))
}

/// `from` dials `to`; `to` must prompt, and answers with `accept`.
async fn dial_and_answer(from: &impl NodeDriver, to: &impl NodeDriver, accept: bool) {
    let (from_id, to_id) = (from.node_id().await, to.node_id().await);
    let dialed = from.connect(&to.ticket().await).await.expect("connect");
    assert_eq!(dialed, to_id);
    let req = expect_event(to, "incoming_request", &from_id, None, "incoming_request").await;
    assert_eq!(req["node_id"], from_id);
    to.respond(&from_id, accept).await.expect("respond");
}

async fn expect_connected(a: &impl NodeDriver, b: &impl NodeDriver) {
    let (a_id, b_id) = (a.node_id().await, b.node_id().await);
    let state = Some("connected");
    expect_event(a, "connection_state", &b_id, state, "a connected").await;
    expect_event(b, "connection_state", &a_id, state, "b connected").await;
    assert_eq!(a.connected().await, vec![b_id]);
    assert_eq!(b.connected().await, vec![a_id]);
}

async fn expect_no_prompt(d: &impl NodeDriver, why: &str) {
    assert!(
        d.wait_event("incoming_request", None, None, QUIET)
            .await
            .is_none(),
        "{why}"
    );
}

/// An unknown peer is prompted exactly once. After Accept it is trusted, so
/// it reconnects with no prompt even after the acceptor restarts.
pub async fn unknown_prompts_once_then_known_reconnects_silently(
    a: &mut impl NodeDriver,
    b: &mut impl NodeDriver,
) {
    let a_id = a.node_id().await;
    dial_and_answer(&*b, &*a, true).await;
    expect_connected(&*a, &*b).await;
    expect_no_prompt(&*a, "accepting must not prompt a second time").await;

    a.restart().await;
    assert_eq!(a.node_id().await, a_id, "identity survives a restart");
    let state = Some("disconnected");
    expect_event(&*b, "connection_state", &a_id, state, "b seeing a leave").await;

    let dialed = b.connect(&a.ticket().await).await.expect("reconnect");
    assert_eq!(dialed, a_id);
    expect_connected(&*a, &*b).await;
    expect_no_prompt(&*a, "a known peer must reconnect without a prompt").await;
}

/// Decline closes the dial with `declined` and trusts nothing: the same peer
/// is prompted again, also after the decliner restarts.
pub async fn decline_leaves_no_trust(a: &mut impl NodeDriver, b: &mut impl NodeDriver) {
    let a_id = a.node_id().await;
    let state = Some("failed");
    dial_and_answer(&*b, &*a, false).await;
    let failed = expect_event(&*b, "connection_state", &a_id, state, "b declined").await;
    assert_eq!(failed["reason"], "declined");
    assert!(a.connected().await.is_empty());
    assert!(b.connected().await.is_empty());

    dial_and_answer(&*b, &*a, false).await;
    expect_event(&*b, "connection_state", &a_id, state, "b declined again").await;

    a.restart().await;
    assert_eq!(a.node_id().await, a_id);
    dial_and_answer(&*b, &*a, true).await;
    expect_connected(&*a, &*b).await;
}

/// Presence defaults to ON, OFF persists across a restart, and a node with
/// presence OFF is still dialable by ticket and still gated by a prompt.
pub async fn presence_off_persists_and_stays_dialable(
    a: &mut impl NodeDriver,
    b: &mut impl NodeDriver,
) {
    assert!(a.presence().await, "presence defaults to ON");
    assert!(!a.set_presence(false).await.expect("presence off"));
    assert!(!a.presence().await);

    a.restart().await;
    assert!(!a.presence().await, "OFF survives a restart");

    dial_and_answer(&*b, &*a, true).await;
    expect_connected(&*a, &*b).await;

    assert!(a.set_presence(true).await.expect("presence on"));
    a.restart().await;
    assert!(a.presence().await, "ON survives a restart");
}

/// One fixed DB key for every account of a scenario.
struct FixedKey([u8; 32]);

impl KeySource for FixedKey {
    fn db_key(&self, _: &AccountId) -> Result<DbKey, KeychainError> {
        Ok(DbKey::from_bytes(self.0))
    }
}

/// Presence goes to the encrypted database so it survives a restart; the rest
/// stays in memory.
struct SqlPresence {
    inner: Arc<MemorySettingsStore>,
    sql: SqlCipherSettingsStore,
}

impl SettingsStore for SqlPresence {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        self.inner.set_display_name(id, name)
    }
    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        self.inner.get_display_name(id)
    }
    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        self.inner.set_provisioned_at(id, unix_ms)
    }
    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        self.inner.get_provisioned_at(id)
    }
    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
        self.sql.get_presence(id)
    }
    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        self.sql.set_presence(id, on)
    }
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        self.inner.get_relay_url(id)
    }
    fn get_theme(&self, id: &AccountId) -> Result<Option<Theme>, SettingsError> {
        self.inner.get_theme(id)
    }
    fn set_theme(&self, id: &AccountId, theme: Theme) -> Result<(), SettingsError> {
        self.inner.set_theme(id, theme)
    }
    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        self.inner.set_relay_url(id, url)
    }
}

type EventBuffer = Arc<Mutex<VecDeque<(&'static str, Value)>>>;

/// A [`NodeDriver`] over an in-process [`TestNode`]. Contacts and presence
/// live in an encrypted database, so they survive [`NodeDriver::restart`].
pub struct InProcessDriver {
    node: Option<TestNode>,
    book: BookCell,
    _store: TempDir,
    events: EventBuffer,
    pump: Option<JoinHandle<()>>,
}

impl InProcessDriver {
    pub async fn new(name: &str) -> Self {
        let store = TempDir::new().expect("temp dir");
        let book = BookCell::new();
        let storage = Arc::new(Storage::new(
            store.path(),
            Arc::new(FixedKey(rand::random())),
        ));
        let contacts = Arc::new(SqlCipherContactStore::new(storage.clone()));
        let known = book.known_factory(contacts, Arc::new(NoMessageStore));
        let wrapper: SettingsWrapper = Arc::new(move |inner| {
            Arc::new(SqlPresence {
                inner,
                sql: SqlCipherSettingsStore::new(storage.clone()),
            })
        });
        let options = TestOptions::default().with_known_factory(known);
        let node = spawn_test_node_wrapped(name, options, Some(wrapper)).await;
        let mut driver = Self {
            node: Some(node),
            book,
            _store: store,
            events: EventBuffer::default(),
            pump: None,
        };
        driver.start_pump();
        driver
    }

    fn node(&self) -> &TestNode {
        self.node.as_ref().expect("node is running")
    }

    fn start_pump(&mut self) {
        let mut rx = self.node().node.subscribe();
        let events = self.events.clone();
        self.pump = Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let mut buf = events.lock().unwrap_or_else(PoisonError::into_inner);
                        if buf.len() == EVENT_BUFFER {
                            buf.pop_front();
                        }
                        buf.push_back(event_json(&ev));
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        }));
    }

    fn take_match(&self, name: &str, node: Option<&str>, state: Option<&str>) -> Option<Value> {
        let mut buf = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        let at = buf.iter().position(|(n, payload)| {
            *n == name
                && node.is_none_or(|id| payload["node_id"] == id)
                && state.is_none_or(|s| payload["state"] == s)
        })?;
        buf.remove(at).map(|(_, payload)| payload)
    }
}

impl NodeDriver for InProcessDriver {
    async fn node_id(&self) -> String {
        self.node().id().to_string()
    }

    async fn ticket(&self) -> String {
        self.node().node.ticket().expect("ticket").to_string()
    }

    async fn connect(&self, ticket: &str) -> Result<String, String> {
        self.node()
            .node
            .connect_ticket(ticket)
            .map(|id| id.to_string())
            .map_err(|e| e.to_string())
    }

    async fn respond(&self, node_id: &str, accept: bool) -> Result<(), String> {
        let peer = node_id.parse().map_err(|_| "invalid node id".to_owned())?;
        let node = &self.node().node;
        if !accept {
            return node
                .respond_incoming(&peer, false)
                .map_err(|e| e.to_string());
        }
        let name = node.pending_request(&peer).and_then(|r| r.display_name);
        let nickname = Nickname::default_for(&peer, name.as_ref());
        accept_incoming(&self.book.get(), node, &peer, nickname.as_str())
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }

    async fn set_presence(&self, on: bool) -> Result<bool, String> {
        let node = &self.node().node;
        node.set_presence(on).map_err(|e| e.to_string())?;
        Ok(node.presence())
    }

    async fn presence(&self) -> bool {
        self.node().node.presence()
    }

    async fn connected(&self) -> Vec<String> {
        self.node()
            .node
            .connected_peers()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    async fn wait_event(
        &self,
        name: &str,
        node: Option<&str>,
        state: Option<&str>,
        timeout: Duration,
    ) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(payload) = self.take_match(name, node, state) {
                return Some(payload);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn restart(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        let old = self.node.take().expect("node is running");
        self.node = Some(restart(old).await);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.start_pump();
    }
}

impl Drop for InProcessDriver {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}
