//! Test doubles for the discovery slice. Everything here drives the same
//! [`Node`] code paths as production; there is no alternate gate.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use iroh::address_lookup::{
    AddressLookup, AddressLookupBuilder, AddressLookupBuilderError, EndpointData, EndpointInfo,
    MemoryLookup,
};
use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, Watcher};
use protocol::ALPN;
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc, Notify};

use crate::discovery::channel::{Channel, ChannelHandler};
use crate::discovery::events::NetEvent;
use crate::discovery::flood::GateLimits;
use crate::discovery::hello::encode_hello;
use crate::discovery::known_peers::{KnownPeers, KnownPeersError, NameSource};
use crate::discovery::lan::{
    encode_lan_name, LanDiscovery, LanError, LanEvent, LanMode, LAN_NAME_MAX_BYTES,
};
use crate::discovery::node::{Node, NodeConfig, NodeDeps};
use crate::discovery::ticket_provider::LiveTicketProvider;
use crate::identity::testing::FakeKeychain;
use crate::identity::{
    AccountId, DisplayName, IdentityService, IdentityStatus, MemorySettingsStore, NodeId,
    SettingsStore,
};

pub mod scenarios;

const LAN_BUS_BUFFER: usize = 2048;

/// One recorded observation: the name (None = cleared) and where it was seen.
pub type ObservedName = (Option<String>, NameSource);

/// In-memory [`KnownPeers`]. Names are advisory observations only.
#[derive(Default)]
pub struct MemoryKnownPeers {
    peers: Mutex<HashSet<NodeId>>,
    observed: Mutex<HashMap<NodeId, Vec<ObservedName>>>,
    dialed: Mutex<Vec<NodeId>>,
}

impl MemoryKnownPeers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Pre-seeds `id` as a trusted peer.
    pub fn trust(&self, id: NodeId) {
        self.lock().insert(id);
    }

    /// Every advisory name observed for the known peer `id`, in order.
    pub fn observed(&self, id: &NodeId) -> Vec<ObservedName> {
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Peers passed to `remember_dialed`, in order.
    pub fn dialed(&self) -> Vec<NodeId> {
        self.dialed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<NodeId>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl KnownPeers for MemoryKnownPeers {
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError> {
        Ok(self.lock().contains(id))
    }

    fn observe(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
        source: NameSource,
    ) -> Result<(), KnownPeersError> {
        if !self.lock().contains(id) {
            return Ok(());
        }
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(*id)
            .or_default()
            .push((name.map(|n| n.as_str().to_owned()), source));
        Ok(())
    }

    fn remember_dialed(
        &self,
        id: &NodeId,
        _name: Option<&DisplayName>,
    ) -> Result<(), KnownPeersError> {
        self.dialed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*id);
        self.lock().insert(*id);
        Ok(())
    }
}

/// A store whose every operation fails, to prove the gate fails closed.
pub struct FailingKnownPeers;

impl KnownPeers for FailingKnownPeers {
    fn is_known(&self, _id: &NodeId) -> Result<bool, KnownPeersError> {
        Err(KnownPeersError)
    }

    fn observe(
        &self,
        _id: &NodeId,
        _name: Option<&DisplayName>,
        _source: NameSource,
    ) -> Result<(), KnownPeersError> {
        Err(KnownPeersError)
    }

    fn remember_dialed(
        &self,
        _id: &NodeId,
        _name: Option<&DisplayName>,
    ) -> Result<(), KnownPeersError> {
        Err(KnownPeersError)
    }
}

/// An in-process stand-in for iroh's internet address lookup. Nodes started
/// with the same instance publish their addresses to it and resolve each
/// other by NodeId.
#[derive(Clone, Debug, Default)]
pub struct MemoryAddressLookup {
    shared: MemoryLookup,
}

impl MemoryAddressLookup {
    pub fn new() -> Self {
        Self::default()
    }

    /// The addresses `id` has published so far.
    pub fn published(&self, id: &NodeId) -> Option<EndpointAddr> {
        self.shared
            .get_endpoint_info(*id)
            .map(EndpointInfo::into_endpoint_addr)
    }
}

impl AddressLookupBuilder for MemoryAddressLookup {
    fn into_address_lookup(
        self,
        endpoint: &Endpoint,
    ) -> Result<impl iroh::address_lookup::AddressLookup, AddressLookupBuilderError> {
        let mut watcher = endpoint.watch_addr();
        let closed = endpoint.closed();
        let shared = self.shared.clone();
        tokio::spawn(async move {
            tokio::pin!(closed);
            loop {
                shared.set_endpoint_info(EndpointInfo::from(watcher.get()));
                tokio::select! {
                    updated = watcher.updated() => {
                        if updated.is_err() {
                            break;
                        }
                    }
                    _ = &mut closed => break,
                }
            }
        });
        Ok(self.shared)
    }
}

/// Records every `EndpointData` the endpoint hands to its address lookups,
/// to prove what would reach internet publishers.
#[derive(Clone, Debug, Default)]
pub struct RecordingAddressLookup {
    published: Arc<Mutex<Vec<EndpointData>>>,
}

impl RecordingAddressLookup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn published(&self) -> Vec<EndpointData> {
        self.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AddressLookup for RecordingAddressLookup {
    fn publish(&self, data: &EndpointData) {
        self.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.clone());
    }
}

struct Member {
    tx: mpsc::Sender<LanEvent>,
}

#[derive(Default)]
struct BusState {
    members: HashMap<NodeId, Member>,
    /// The record each advertising node currently publishes.
    advertised: HashMap<NodeId, (EndpointAddr, Option<Vec<u8>>)>,
    advertise_calls: HashMap<NodeId, usize>,
}

/// A deterministic in-process stand-in for the multicast socket. Advertising
/// nodes are visible to every other node on the same bus.
#[derive(Clone, Default)]
pub struct MemoryLanBus {
    state: Arc<Mutex<BusState>>,
}

impl MemoryLanBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BusState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(in crate::discovery) fn join(&self, endpoint: Endpoint) -> MemoryLan {
        let id = endpoint.id();
        let (tx, rx) = mpsc::channel(LAN_BUS_BUFFER);
        let mut state = self.lock();
        for (other, (addr, user_data)) in &state.advertised {
            if *other != id {
                let _ = tx.try_send(LanEvent::Discovered {
                    addr: addr.clone(),
                    user_data: user_data.clone(),
                });
            }
        }
        state.members.insert(id, Member { tx });
        MemoryLan {
            bus: self.clone(),
            endpoint,
            events: Mutex::new(Some(rx)),
        }
    }

    /// What `id` currently publishes: `None` when not advertising, otherwise
    /// the raw name bytes attached to its record (`None` inside = no name).
    pub fn advertised_record(&self, id: &NodeId) -> Option<Option<Vec<u8>>> {
        self.lock().advertised.get(id).map(|(_, data)| data.clone())
    }

    /// How many times `id` turned its record on or refreshed it on this bus.
    pub fn advertise_count(&self, id: &NodeId) -> usize {
        self.lock().advertise_calls.get(id).copied().unwrap_or(0)
    }

    /// Feeds an arbitrary observation to `target`, through the same registry
    /// and node code as a real one.
    pub async fn inject(&self, target: &NodeId, ev: LanEvent) {
        let tx = self.lock().members.get(target).map(|m| m.tx.clone());
        if let Some(tx) = tx {
            let _ = tx.send(ev).await;
        }
    }
}

/// One node's handle on a [`MemoryLanBus`].
pub struct MemoryLan {
    bus: MemoryLanBus,
    endpoint: Endpoint,
    events: Mutex<Option<mpsc::Receiver<LanEvent>>>,
}

impl LanDiscovery for MemoryLan {
    fn set_advertise(&self, on: bool, local_name: Option<&DisplayName>) -> Result<(), LanError> {
        let id = self.endpoint.id();
        let mut state = self.bus.lock();
        let event = if on {
            let addr = self.endpoint.addr();
            let user_data = local_name.map(|n| encode_lan_name(n, LAN_NAME_MAX_BYTES));
            *state.advertise_calls.entry(id).or_default() += 1;
            state
                .advertised
                .insert(id, (addr.clone(), user_data.clone()));
            LanEvent::Discovered { addr, user_data }
        } else if state.advertised.remove(&id).is_some() {
            LanEvent::Expired(id)
        } else {
            return Ok(());
        };
        for (other, member) in &state.members {
            if *other != id {
                let _ = member.tx.try_send(event.clone());
            }
        }
        Ok(())
    }

    fn events(&self) -> mpsc::Receiver<LanEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
}

/// Wraps the node's settings store, to inject faults or a different backing
/// store for presence.
pub type SettingsWrapper =
    Arc<dyn Fn(Arc<MemorySettingsStore>) -> Arc<dyn SettingsStore + Send + Sync> + Send + Sync>;

/// Builds the known-peers store once the account and NodeId are provisioned.
pub type KnownFactory = Arc<dyn Fn(AccountId, NodeId) -> Arc<dyn KnownPeers> + Send + Sync>;

/// How a test node is configured beyond the defaults.
#[derive(Clone)]
pub struct TestOptions {
    pub config: NodeConfig,
    pub known: Arc<dyn KnownPeers>,
    known_factory: Option<KnownFactory>,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            config: NodeConfig::loopback(),
            known: Arc::new(MemoryKnownPeers::new()),
            known_factory: None,
        }
    }
}

impl TestOptions {
    pub fn with_limits(mut self, limits: GateLimits) -> Self {
        self.config.limits = limits;
        self
    }

    pub fn with_lookup(mut self, lookup: &MemoryAddressLookup) -> Self {
        self.config = self.config.with_lookup(lookup.clone());
        self
    }

    pub fn with_known(mut self, known: Arc<dyn KnownPeers>) -> Self {
        self.known = known;
        self
    }

    /// Builds the store after provisioning, from the account and NodeId; runs
    /// again on `restart`, so it must capture a shared store to keep data.
    pub fn with_known_factory(mut self, f: KnownFactory) -> Self {
        self.known_factory = Some(f);
        self
    }

    pub fn with_lan(mut self, bus: &MemoryLanBus) -> Self {
        self.config = self.config.with_lan(LanMode::Memory(bus.clone()));
        self
    }

    pub fn with_recorder(mut self, recorder: &RecordingAddressLookup) -> Self {
        self.config = self.config.with_recorder(recorder.clone());
        self
    }
}

pub type TestIdentity =
    IdentityService<Arc<FakeKeychain>, Arc<MemorySettingsStore>, LiveTicketProvider>;

/// A provisioned identity plus a running node, in an isolated directory.
pub struct TestNode {
    pub node: Node,
    pub identity: TestIdentity,
    pub known: Arc<dyn KnownPeers>,
    pub dir: TempDir,
    pub keychain: Arc<FakeKeychain>,
    settings: Arc<MemorySettingsStore>,
    tickets: LiveTicketProvider,
    options: TestOptions,
    wrapper: Option<SettingsWrapper>,
}

impl TestNode {
    pub fn id(&self) -> NodeId {
        self.node.node_id()
    }
}

pub async fn spawn_test_node(name: &str) -> TestNode {
    spawn_test_node_with(name, TestOptions::default()).await
}

pub async fn spawn_test_node_with(name: &str, options: TestOptions) -> TestNode {
    spawn_test_node_wrapped(name, options, None).await
}

/// Like [`spawn_test_node_with`], with the node reading its settings through
/// `wrapper` (to inject faults or back presence with a file).
pub async fn spawn_test_node_wrapped(
    name: &str,
    options: TestOptions,
    wrapper: Option<SettingsWrapper>,
) -> TestNode {
    let dir = TempDir::new().expect("temp dir");
    let keychain = Arc::new(FakeKeychain::new());
    let settings = Arc::new(MemorySettingsStore::new());
    let tickets = LiveTicketProvider::new();
    let identity = IdentityService::new(
        keychain.clone(),
        settings.clone(),
        tickets.clone(),
        dir.path(),
    );
    identity.provision().expect("provision");
    identity.set_display_name(name).expect("display name");
    start(identity, dir, keychain, settings, tickets, options, wrapper).await
}

/// Stops the node and starts a new one over the same keychain, settings,
/// directory and known-peers store.
pub async fn restart(old: TestNode) -> TestNode {
    let TestNode {
        node,
        identity,
        dir,
        keychain,
        settings,
        tickets,
        options,
        wrapper,
        ..
    } = old;
    node.shutdown().await;
    start(identity, dir, keychain, settings, tickets, options, wrapper).await
}

async fn start(
    identity: TestIdentity,
    dir: TempDir,
    keychain: Arc<FakeKeychain>,
    settings: Arc<MemorySettingsStore>,
    tickets: LiveTicketProvider,
    options: TestOptions,
    wrapper: Option<SettingsWrapper>,
) -> TestNode {
    let known = match &options.known_factory {
        Some(factory) => {
            let IdentityStatus::Present {
                account_id,
                node_id,
            } = identity.status()
            else {
                panic!("identity must be provisioned before the known factory runs");
            };
            factory(account_id, node_id)
        }
        None => options.known.clone(),
    };
    let settings_dyn: Arc<dyn SettingsStore + Send + Sync> = match &wrapper {
        Some(wrap) => wrap(settings.clone()),
        None => settings.clone(),
    };
    let node = Node::start(
        &identity,
        NodeDeps {
            settings: settings_dyn,
            known: known.clone(),
            tickets: tickets.clone(),
        },
        options.config.clone(),
    )
    .await
    .expect("node start");
    TestNode {
        node,
        identity,
        known,
        dir,
        keychain,
        settings,
        tickets,
        options,
        wrapper,
    }
}

#[derive(Default)]
struct Recorded {
    seen: Vec<(NodeId, usize)>,
    channels: Vec<Channel>,
    queried: Vec<bool>,
}

/// A [`ChannelHandler`] that records every delivery and keeps the channels.
/// [`RecordingChannelHandler::querying`] also calls `Node::channel` from inside
/// `on_channel`, which proves the node never calls the handler under a lock.
pub struct RecordingChannelHandler {
    node: Option<Weak<Node>>,
    recorded: Mutex<Recorded>,
    changed: Notify,
}

impl RecordingChannelHandler {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            node: None,
            recorded: Mutex::default(),
            changed: Notify::new(),
        })
    }

    pub fn querying(node: &Arc<Node>) -> Arc<Self> {
        Arc::new(Self {
            node: Some(Arc::downgrade(node)),
            recorded: Mutex::default(),
            changed: Notify::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every `(peer, stable_id)` delivered, in order.
    pub fn seen(&self) -> Vec<(NodeId, usize)> {
        self.lock().seen.clone()
    }

    pub fn channels(&self) -> Vec<Channel> {
        self.lock().channels.clone()
    }

    /// For each delivery, whether `Node::channel` returned that channel from
    /// inside `on_channel` (querying handlers only).
    pub fn queried(&self) -> Vec<bool> {
        self.lock().queried.clone()
    }

    /// The first channel delivered for `peer`, waiting up to `within`.
    pub async fn wait_for(&self, peer: &NodeId, within: Duration) -> Option<Channel> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let changed = self.changed.notified();
            if let Some(found) = self.lock().channels.iter().find(|c| c.peer() == *peer) {
                return Some(found.clone());
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return None;
            }
        }
    }
}

impl ChannelHandler for RecordingChannelHandler {
    fn on_channel(&self, channel: Channel) {
        let queried = self.node.as_ref().map(|weak| {
            weak.upgrade()
                .and_then(|node| node.channel(&channel.peer()))
                .is_some_and(|c| c.stable_id() == channel.stable_id())
        });
        let mut rec = self.lock();
        rec.seen.push((channel.peer(), channel.stable_id()));
        rec.queried.extend(queried);
        rec.channels.push(channel);
        drop(rec);
        self.changed.notify_waiters();
    }
}

/// The next event within `timeout`, skipping lag notices.
pub async fn next_event(
    rx: &mut broadcast::Receiver<NetEvent>,
    timeout: Duration,
) -> Option<NetEvent> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => return Some(event),
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
        }
    }
}

/// The bytes of a valid Hello frame announcing `name`.
pub fn hello_frame(name: &str) -> Vec<u8> {
    encode_hello(name)
}

/// A bare iroh endpoint with a fresh identity that has dialed a node and sent
/// arbitrary bytes. Dropping it ends the connection.
pub struct RawDial {
    pub conn: Connection,
    // Kept open so the acceptor's Hello reply is not refused mid-write.
    _recv: iroh::endpoint::RecvStream,
    _endpoint: Endpoint,
}

pub async fn raw_dial(addr: EndpointAddr, first_frame: &[u8]) -> Result<RawDial, String> {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .relay_mode(RelayMode::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|e| e.to_string())?
        .bind()
        .await
        .map_err(|e| e.to_string())?;
    let conn = endpoint
        .connect(addr, ALPN)
        .await
        .map_err(|e| e.to_string())?;
    let (mut send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    // The acceptor may close mid-write on oversized input; that is the test.
    let _ = send.write_all(first_frame).await;
    let _ = send.finish();
    Ok(RawDial {
        conn,
        _recv: recv,
        _endpoint: endpoint,
    })
}
