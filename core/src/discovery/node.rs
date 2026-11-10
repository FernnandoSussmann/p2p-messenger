//! The live node: one iroh endpoint bound to the identity key, the `Hello`
//! exchange and the NodeId-based trust gate.
//!
//! Every inbound connection is judged on `conn.remote_id()`, the
//! transport-verified NodeId. Names from `Hello` are advisory. While an
//! unknown peer awaits Accept/Decline nothing is read from or sent to it past
//! its first frame, and it is not in [`Node::connected_peers`].

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, Connection, ConnectionError, Incoming, VarInt};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey, Watcher};
use protocol::{ALPN, MAX_HELLO_BYTES};
use tokio::sync::{broadcast, oneshot};
use tokio::time::{sleep, timeout};

use super::events::{ConnectionState, Direction, FailureReason, IncomingRequest, NetEvent};
use super::flood::{Admission, FloodGuard, GateLimits};
use super::hello::{decode_hello, encode_hello};
use super::known_peers::KnownPeers;
use super::lan::{LanDiscovery, LanError, LanEvent, LanMode, LanPeer, LanPeers, MdnsLan, NoLan};
use super::ticket_provider::LiveTicketProvider;
use crate::identity::{
    AccountId, DisplayName, IdentityService, KeychainStore, LoadError, NodeId, SettingsError,
    SettingsStore, Ticket, TicketError, TicketProvider,
};

/// The peer declined the request.
pub const CLOSE_DECLINED: u32 = 1;
/// The peer broke the protocol (bad first frame, oversize, timeout).
pub const CLOSE_PROTOCOL: u32 = 2;
/// The acceptor is rate limiting; no prompt was raised.
pub const CLOSE_BUSY: u32 = 3;
const CLOSE_REPLACED: u32 = 4;
const CLOSE_NORMAL: u32 = 0;

const EVENT_CAPACITY: usize = 256;
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
const REPLY_GRACE: Duration = Duration::from_secs(10);
const TICKET_CHANGE_DEBOUNCE: Duration = Duration::from_secs(2);
const LAN_EVENT_DEBOUNCE: Duration = Duration::from_millis(250);

/// Which relays the endpoint uses.
#[derive(Debug, Clone)]
pub enum RelayChoice {
    /// The default n0 relay mesh.
    Default,
    Disabled,
    Custom(RelayUrl),
}

/// How a node binds and gates. Use [`NodeConfig::desktop`] in the app.
#[derive(Clone)]
pub struct NodeConfig {
    pub relay: RelayChoice,
    pub bind_addr: Option<SocketAddr>,
    pub limits: GateLimits,
    pub lan: LanMode,
    internet_lookup: bool,
    #[cfg(any(test, feature = "test-support"))]
    relay_only: bool,
    #[cfg(any(test, feature = "test-support"))]
    recorder: Option<super::testing::RecordingAddressLookup>,
    #[cfg(any(test, feature = "test-support"))]
    lookup: Option<super::testing::MemoryAddressLookup>,
    #[cfg(any(test, feature = "test-support"))]
    ca_tls: Option<iroh::tls::CaTlsConfig>,
}

impl NodeConfig {
    /// iroh's N0 preset: default relay mesh plus internet address lookup,
    /// giving hole punching with relay fallback.
    pub fn desktop() -> Self {
        Self {
            relay: RelayChoice::Default,
            bind_addr: None,
            limits: GateLimits::default(),
            lan: LanMode::Mdns,
            internet_lookup: true,
            #[cfg(any(test, feature = "test-support"))]
            relay_only: false,
            #[cfg(any(test, feature = "test-support"))]
            recorder: None,
            #[cfg(any(test, feature = "test-support"))]
            lookup: None,
            #[cfg(any(test, feature = "test-support"))]
            ca_tls: None,
        }
    }

    /// No relay, no internet lookup, bound to `127.0.0.1:0`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback() -> Self {
        Self {
            relay: RelayChoice::Disabled,
            bind_addr: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            lan: LanMode::Off,
            internet_lookup: false,
            ..Self::desktop()
        }
    }

    /// A custom relay with the IP transports cleared, so traffic can only
    /// take the relay path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn relay_only(relay: RelayUrl) -> Self {
        Self {
            relay: RelayChoice::Custom(relay),
            bind_addr: None,
            lan: LanMode::Off,
            internet_lookup: false,
            relay_only: true,
            ..Self::desktop()
        }
    }

    pub fn with_lan(mut self, lan: LanMode) -> Self {
        self.lan = lan;
        self
    }

    /// Registers a lookup that records every `EndpointData` the endpoint
    /// publishes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_recorder(mut self, recorder: super::testing::RecordingAddressLookup) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Registers a shared in-process address lookup.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_lookup(mut self, lookup: super::testing::MemoryAddressLookup) -> Self {
        self.lookup = Some(lookup);
        self
    }

    /// Overrides relay TLS verification, for a self-signed test relay.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_ca_tls(mut self, ca_tls: iroh::tls::CaTlsConfig) -> Self {
        self.ca_tls = Some(ca_tls);
        self
    }

    fn expects_relay(&self) -> bool {
        !matches!(self.relay, RelayChoice::Disabled)
    }
}

/// What [`Node::start`] needs besides the identity.
pub struct NodeDeps {
    /// Read for our own display name at Hello send time.
    pub settings: Arc<dyn SettingsStore + Send + Sync>,
    pub known: Arc<dyn KnownPeers>,
    /// The node attaches itself on start.
    pub tickets: LiveTicketProvider,
}

#[derive(Debug)]
pub enum StartError {
    Identity(LoadError),
    Bind(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(e) => write!(f, "identity unavailable: {e}"),
            Self::Bind(e) => write!(f, "could not start the endpoint: {e}"),
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Identity(e) => Some(e),
            Self::Bind(_) => None,
        }
    }
}

/// Why presence could not be changed. Advertising is left as it was.
#[derive(Debug)]
pub enum PresenceError {
    Settings(SettingsError),
    Lan(LanError),
}

impl fmt::Display for PresenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(e) => write!(f, "could not save presence: {e}"),
            Self::Lan(e) => write!(f, "could not change presence: {e}"),
        }
    }
}

impl std::error::Error for PresenceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    InvalidTicket,
    SelfDial,
    ShuttingDown,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTicket => "invalid ticket",
            Self::SelfDial => "cannot connect to yourself",
            Self::ShuttingDown => "the node is shutting down",
        })
    }
}

impl std::error::Error for ConnectError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespondError {
    NoSuchRequest,
}

impl fmt::Display for RespondError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no such pending request")
    }
}

impl std::error::Error for RespondError {}

struct Pending {
    request: IncomingRequest,
    decision: oneshot::Sender<bool>,
}

struct Authorized {
    conn: Connection,
    stable_id: usize,
}

struct State {
    flood: FloodGuard,
    handshakes: usize,
    pending: HashMap<NodeId, Pending>,
    authorized: HashMap<NodeId, Authorized>,
}

struct Inner {
    endpoint: Endpoint,
    node_id: NodeId,
    account: AccountId,
    settings: Arc<dyn SettingsStore + Send + Sync>,
    known: Arc<dyn KnownPeers>,
    tickets: LiveTicketProvider,
    limits: GateLimits,
    expects_relay: bool,
    events: broadcast::Sender<NetEvent>,
    lan: Arc<dyn LanDiscovery>,
    lan_peers: Mutex<LanPeers>,
    /// Whether our LAN record is published. Held while it changes.
    presence: Mutex<bool>,
    state: Mutex<State>,
    closing: AtomicBool,
    runtime: tokio::runtime::Handle,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: NetEvent) {
        // No receivers is fine; a lagging one loses old events, never blocks.
        let _ = self.events.send(event);
    }

    fn emit_state(&self, node_id: NodeId, direction: Direction, state: ConnectionState) {
        self.emit(NetEvent::ConnectionState {
            node_id,
            direction,
            state,
        });
    }

    fn local_name(&self) -> Option<DisplayName> {
        self.settings.get_display_name(&self.account).ok().flatten()
    }

    fn own_hello(&self) -> Vec<u8> {
        encode_hello(self.local_name().as_ref().map_or("", DisplayName::as_str))
    }

    fn take_pending(&self, peer: &NodeId) -> Option<Pending> {
        let mut state = self.state();
        let pending = state.pending.remove(peer);
        if pending.is_some() {
            state.flood.resolve(peer);
        }
        pending
    }
}

/// The running node. Dropping it closes the endpoint.
pub struct Node {
    inner: Arc<Inner>,
}

impl Node {
    pub async fn start<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
        identity: &IdentityService<K, S, T>,
        deps: NodeDeps,
        config: NodeConfig,
    ) -> Result<Node, StartError> {
        let (account, secret) = identity
            .load_identity_secret()
            .map_err(StartError::Identity)?;
        let key = SecretKey::from_bytes(secret.as_bytes());
        drop(secret);
        let endpoint = build_endpoint(key, &config).await?;
        let node_id = endpoint.id();
        let lan: Arc<dyn LanDiscovery> = match &config.lan {
            LanMode::Mdns => match MdnsLan::start(endpoint.clone()).await {
                Ok(mdns) => Arc::new(mdns),
                Err(e) => {
                    tracing::warn!(error = %e, "LAN discovery unavailable");
                    Arc::new(NoLan)
                }
            },
            LanMode::Off => Arc::new(NoLan),
            #[cfg(any(test, feature = "test-support"))]
            LanMode::Memory(bus) => Arc::new(bus.join(endpoint.clone())),
        };
        let lan_events = lan.events();
        // A store that cannot be read must not make us visible.
        let presence = match deps.settings.get_presence(&account) {
            Ok(saved) => saved.unwrap_or(true),
            Err(e) => {
                tracing::warn!(error = %e, "presence unreadable; staying hidden");
                false
            }
        };
        if presence {
            let name = deps.settings.get_display_name(&account).ok().flatten();
            if let Err(e) = lan.set_advertise(true, name.as_ref()) {
                tracing::warn!(error = %e, "could not advertise on the LAN");
            }
        }
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let inner = Arc::new(Inner {
            endpoint: endpoint.clone(),
            node_id,
            account,
            settings: deps.settings,
            known: deps.known,
            tickets: deps.tickets,
            limits: config.limits.clone(),
            expects_relay: config.expects_relay(),
            events,
            lan,
            lan_peers: Mutex::new(LanPeers::new(node_id)),
            presence: Mutex::new(presence),
            state: Mutex::new(State {
                flood: FloodGuard::new(config.limits.clone()),
                handshakes: 0,
                pending: HashMap::new(),
                authorized: HashMap::new(),
            }),
            closing: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
        });
        inner.tickets.attach(endpoint);
        inner.runtime.spawn(accept_loop(inner.clone()));
        inner.runtime.spawn(watch_ticket(inner.clone()));
        inner.runtime.spawn(lan_loop(inner.clone(), lan_events));
        tracing::debug!(node = %node_id.fmt_short(), "node started");
        Ok(Node { inner })
    }

    pub fn node_id(&self) -> NodeId {
        self.inner.node_id
    }

    /// A ticket from the endpoint's addresses as they are right now.
    pub fn ticket(&self) -> Result<Ticket, TicketError> {
        Ticket::from_endpoint_addr(&self.inner.endpoint.addr())
    }

    /// Waits up to `timeout` for the endpoint to know how it can be reached
    /// (a home relay, or a direct address when no relay is configured), then
    /// returns the ticket. On timeout it returns the best ticket available.
    pub async fn ticket_when_ready(&self, timeout_after: Duration) -> Result<Ticket, TicketError> {
        let endpoint = self.inner.endpoint.clone();
        let expects_relay = self.inner.expects_relay;
        let ready = move |addr: &EndpointAddr| {
            if expects_relay {
                addr.relay_urls().next().is_some()
            } else {
                addr.ip_addrs().next().is_some()
            }
        };
        let _ = timeout(timeout_after, async move {
            let mut watcher = endpoint.watch_addr();
            while !ready(&watcher.get()) {
                if watcher.updated().await.is_err() {
                    break;
                }
            }
        })
        .await;
        self.ticket()
    }

    /// Validates the ticket and starts dialing in the background. Progress
    /// arrives as [`NetEvent`]s.
    pub fn connect_ticket(&self, input: &str) -> Result<NodeId, ConnectError> {
        if self.inner.closing.load(Ordering::SeqCst) {
            return Err(ConnectError::ShuttingDown);
        }
        let parsed = Ticket::parse(input).map_err(|_| ConnectError::InvalidTicket)?;
        if parsed.node_id == self.inner.node_id {
            return Err(ConnectError::SelfDial);
        }
        let peer = parsed.node_id;
        self.inner
            .runtime
            .spawn(dial(self.inner.clone(), parsed.addr));
        Ok(peer)
    }

    /// Answers the open prompt of `peer`.
    pub fn respond_incoming(&self, peer: &NodeId, accept: bool) -> Result<(), RespondError> {
        let pending = self
            .inner
            .take_pending(peer)
            .ok_or(RespondError::NoSuchRequest)?;
        // The prompt task may have just ended; then there is nothing to answer.
        pending
            .decision
            .send(accept)
            .map_err(|_| RespondError::NoSuchRequest)
    }

    /// Snapshot of open prompts, for seeding the UI after mount or reload.
    pub fn pending_incoming(&self) -> Vec<IncomingRequest> {
        self.inner
            .state()
            .pending
            .values()
            .map(|p| p.request.clone())
            .collect()
    }

    /// Whether our LAN record is published.
    pub fn presence(&self) -> bool {
        *self
            .inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Persists the choice, then applies it. If either step fails the
    /// advertising state and the saved value stay as they were. Browsing,
    /// accepting and dialing are never affected.
    pub fn set_presence(&self, on: bool) -> Result<(), PresenceError> {
        let inner = &self.inner;
        let mut current = inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = *current;
        inner
            .settings
            .set_presence(&inner.account, on)
            .map_err(PresenceError::Settings)?;
        let name = if on { inner.local_name() } else { None };
        if let Err(e) = inner.lan.set_advertise(on, name.as_ref()) {
            if let Err(restore) = inner.settings.set_presence(&inner.account, previous) {
                tracing::warn!(error = %restore, "could not restore the saved presence");
            }
            return Err(PresenceError::Lan(e));
        }
        *current = on;
        Ok(())
    }

    /// Peers currently seen on the LAN, sorted by NodeId. Memory only.
    pub fn lan_peers(&self) -> Vec<LanPeer> {
        self.inner
            .lan_peers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot()
    }

    /// Re-publishes our name on the LAN after a rename. Does nothing while
    /// presence is OFF.
    pub fn refresh_local_name(&self) {
        let inner = &self.inner;
        let current = inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *current {
            if let Err(e) = inner.lan.set_advertise(true, inner.local_name().as_ref()) {
                tracing::warn!(error = %e, "could not refresh the LAN name");
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NetEvent> {
        self.inner.events.subscribe()
    }

    /// Authorized channels only.
    pub fn connected_peers(&self) -> Vec<NodeId> {
        self.inner.state().authorized.keys().copied().collect()
    }

    pub async fn shutdown(self) {
        self.inner.close().await;
    }

    /// True when every open path to `peer` goes through a relay.
    #[cfg(any(test, feature = "test-support"))]
    pub fn connection_is_relay_only(&self, peer: &NodeId) -> Option<bool> {
        let conn = self.inner.state().authorized.get(peer)?.conn.clone();
        let paths = conn.paths();
        Some(!paths.is_empty() && paths.iter().all(|p| p.is_relay()))
    }

    /// Adds an external address to the endpoint, which changes its ticket.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn add_test_external_addr(&self, addr: SocketAddr) {
        self.inner.endpoint.add_external_addr(addr).await;
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let inner = self.inner.clone();
        self.inner.runtime.spawn(async move { inner.close().await });
    }
}

impl Inner {
    async fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.tickets.detach();
        if let Err(e) = self.lan.set_advertise(false, None) {
            tracing::debug!(error = %e, "could not withdraw the LAN record");
        }
        {
            let mut state = self.state();
            state.pending.clear();
            state.authorized.clear();
        }
        self.endpoint.close().await;
    }
}

async fn build_endpoint(key: SecretKey, config: &NodeConfig) -> Result<Endpoint, StartError> {
    let bind_err = |e: &dyn fmt::Display| StartError::Bind(e.to_string());
    let mut builder = Endpoint::builder(presets::Minimal);
    if config.internet_lookup {
        builder = builder.preset(presets::N0);
    }
    builder = builder.secret_key(key).alpns(vec![ALPN.to_vec()]);
    builder = builder.relay_mode(match &config.relay {
        RelayChoice::Default => RelayMode::Default,
        RelayChoice::Disabled => RelayMode::Disabled,
        RelayChoice::Custom(url) => RelayMode::custom([url.clone()]),
    });
    if let Some(addr) = config.bind_addr {
        builder = builder.bind_addr(addr).map_err(|e| bind_err(&e))?;
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        if config.relay_only {
            builder = builder.clear_ip_transports();
        }
        if let Some(lookup) = &config.lookup {
            builder = builder.address_lookup(lookup.clone());
        }
        if let Some(recorder) = &config.recorder {
            builder = builder.address_lookup(recorder.clone());
        }
        if let Some(ca_tls) = &config.ca_tls {
            builder = builder.ca_tls_config(ca_tls.clone());
        }
    }
    builder.bind().await.map_err(|e| bind_err(&e))
}

/// Holds one of the `max_handshakes` slots until dropped.
struct HandshakeSlot(Arc<Inner>);

impl HandshakeSlot {
    fn acquire(inner: &Arc<Inner>) -> Option<Self> {
        let mut state = inner.state();
        if state.handshakes >= inner.limits.max_handshakes {
            return None;
        }
        state.handshakes += 1;
        Some(Self(inner.clone()))
    }
}

impl Drop for HandshakeSlot {
    fn drop(&mut self) {
        let mut state = self.0.state();
        state.handshakes = state.handshakes.saturating_sub(1);
    }
}

async fn accept_loop(inner: Arc<Inner>) {
    while let Some(incoming) = inner.endpoint.accept().await {
        let Some(slot) = HandshakeSlot::acquire(&inner) else {
            incoming.refuse();
            continue;
        };
        inner
            .runtime
            .spawn(handle_incoming(inner.clone(), incoming, slot));
    }
}

fn close(conn: &Connection, code: u32) {
    conn.close(VarInt::from_u32(code), b"");
}

async fn handle_incoming(inner: Arc<Inner>, incoming: Incoming, slot: HandshakeSlot) {
    let Ok(accepting) = incoming.accept() else {
        return;
    };
    let hello_timeout = inner.limits.hello_timeout;
    let Ok(Ok(conn)) = timeout(hello_timeout, accepting).await else {
        return;
    };
    let peer = conn.remote_id();

    let first_frame = timeout(hello_timeout, async {
        let (send, mut recv) = conn.accept_bi().await.ok()?;
        let bytes = recv.read_to_end(MAX_HELLO_BYTES).await.ok()?;
        Some((send, bytes))
    })
    .await;
    let Ok(Some((send, bytes))) = first_frame else {
        tracing::debug!(peer = %peer.fmt_short(), "no valid first frame");
        close(&conn, CLOSE_PROTOCOL);
        return;
    };
    let Ok(name) = decode_hello(&bytes) else {
        tracing::debug!(peer = %peer.fmt_short(), "first frame rejected");
        close(&conn, CLOSE_PROTOCOL);
        return;
    };
    drop(slot);

    // A storage error is treated as "unknown": fail closed, prompt the user.
    if matches!(inner.known.is_known(&peer), Ok(true)) {
        reply_and_authorize(&inner, conn, send, peer).await;
        return;
    }

    let (decision_tx, mut decision_rx) = oneshot::channel();
    {
        let mut state = inner.state();
        if state.flood.admit(peer, Instant::now()) == Admission::Drop {
            drop(state);
            close(&conn, CLOSE_BUSY);
            return;
        }
        let request = IncomingRequest {
            node_id: peer,
            display_name: name.clone(),
        };
        state.pending.insert(
            peer,
            Pending {
                request: request.clone(),
                decision: decision_tx,
            },
        );
        inner.emit(NetEvent::IncomingRequest(request));
    }

    let outcome = tokio::select! {
        answer = &mut decision_rx => Outcome::from_answer(answer.ok()),
        _ = conn.closed() => settle(&inner, &peer, &mut decision_rx, Outcome::Closed).await,
        _ = sleep(inner.limits.prompt_ttl) => settle(&inner, &peer, &mut decision_rx, Outcome::Expired).await,
    };

    match outcome {
        Outcome::Accepted => {
            if let Err(e) = inner.known.add(peer, name) {
                tracing::debug!(peer = %peer.fmt_short(), error = %e, "could not remember peer");
            }
            reply_and_authorize(&inner, conn, send, peer).await;
        }
        Outcome::Declined => {
            close(&conn, CLOSE_DECLINED);
            inner.emit_state(
                peer,
                Direction::Inbound,
                ConnectionState::Failed(FailureReason::Declined),
            );
        }
        Outcome::Expired => {
            close(&conn, CLOSE_DECLINED);
            inner.emit_state(
                peer,
                Direction::Inbound,
                ConnectionState::Failed(FailureReason::Expired),
            );
        }
        Outcome::Closed => inner.emit_state(
            peer,
            Direction::Inbound,
            ConnectionState::Failed(FailureReason::Closed),
        ),
        Outcome::Shutdown => close(&conn, CLOSE_NORMAL),
    }
}

enum Outcome {
    Accepted,
    Declined,
    Expired,
    Closed,
    Shutdown,
}

impl Outcome {
    fn from_answer(answer: Option<bool>) -> Self {
        match answer {
            Some(true) => Self::Accepted,
            Some(false) => Self::Declined,
            None => Self::Shutdown,
        }
    }
}

/// The prompt ended on its own (connection gone or TTL). If the user answered
/// in the same instant, the answer wins; otherwise the prompt is removed.
async fn settle(
    inner: &Inner,
    peer: &NodeId,
    decision_rx: &mut oneshot::Receiver<bool>,
    fallback: Outcome,
) -> Outcome {
    if inner.take_pending(peer).is_some() {
        fallback
    } else {
        Outcome::from_answer(decision_rx.await.ok())
    }
}

/// Writes our Hello on the accepted stream, then marks the peer authorized.
async fn reply_and_authorize(
    inner: &Arc<Inner>,
    conn: Connection,
    mut send: iroh::endpoint::SendStream,
    peer: NodeId,
) {
    let sent = async {
        send.write_all(&inner.own_hello()).await.ok()?;
        send.finish().ok()
    }
    .await;
    if sent.is_none() {
        close(&conn, CLOSE_NORMAL);
        inner.emit_state(
            peer,
            Direction::Inbound,
            ConnectionState::Failed(FailureReason::Closed),
        );
        return;
    }
    authorize(inner, conn, peer, Direction::Inbound);
}

/// Records an authorized channel, replacing any older one for the same peer,
/// and reports its end as `Disconnected`.
fn authorize(inner: &Arc<Inner>, conn: Connection, peer: NodeId, direction: Direction) {
    let stable_id = conn.stable_id();
    let replaced = inner.state().authorized.insert(
        peer,
        Authorized {
            conn: conn.clone(),
            stable_id,
        },
    );
    if let Some(old) = replaced {
        close(&old.conn, CLOSE_REPLACED);
    }
    inner.emit_state(peer, direction, ConnectionState::Connected);

    let inner = inner.clone();
    inner.runtime.clone().spawn(async move {
        conn.closed().await;
        let removed = {
            let mut state = inner.state();
            let current = state.authorized.get(&peer).map(|a| a.stable_id);
            if current == Some(stable_id) {
                state.authorized.remove(&peer);
                true
            } else {
                false
            }
        };
        if removed {
            inner.emit_state(peer, direction, ConnectionState::Disconnected);
        }
    });
}

async fn dial(inner: Arc<Inner>, addr: EndpointAddr) {
    let peer = addr.id;
    inner.emit_state(peer, Direction::Outbound, ConnectionState::Connecting);
    if let Err(reason) = run_dial(&inner, addr).await {
        inner.emit_state(peer, Direction::Outbound, ConnectionState::Failed(reason));
    }
}

async fn run_dial(inner: &Arc<Inner>, addr: EndpointAddr) -> Result<(), FailureReason> {
    let peer = addr.id;
    let conn = match timeout(DIAL_TIMEOUT, inner.endpoint.connect(addr, ALPN)).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(_)) => return Err(FailureReason::Unreachable),
        Err(_) => return Err(FailureReason::Timeout),
    };
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|_| FailureReason::Unreachable)?;
    let sent = async {
        send.write_all(&inner.own_hello()).await.ok()?;
        send.finish().ok()
    }
    .await;
    if sent.is_none() {
        return Err(close_reason(&conn));
    }

    let reply_wait = inner.limits.prompt_ttl + REPLY_GRACE;
    let reply = match timeout(reply_wait, recv.read_to_end(MAX_HELLO_BYTES)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return Err(close_reason(&conn)),
        Err(_) => {
            close(&conn, CLOSE_NORMAL);
            return Err(FailureReason::Timeout);
        }
    };
    let Ok(name) = decode_hello(&reply) else {
        close(&conn, CLOSE_PROTOCOL);
        return Err(FailureReason::Protocol);
    };
    // The user deliberately dialed this NodeId: remember it, but only now
    // that it answered with a valid Hello.
    if let Err(e) = inner.known.add(peer, name) {
        tracing::debug!(peer = %peer.fmt_short(), error = %e, "could not remember peer");
    }
    authorize(inner, conn, peer, Direction::Outbound);
    Ok(())
}

/// Why a connection we dialed ended before it was authorized.
fn close_reason(conn: &Connection) -> FailureReason {
    match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(close)) => match u64::from(close.error_code) {
            c if c == u64::from(CLOSE_DECLINED) => FailureReason::Declined,
            c if c == u64::from(CLOSE_PROTOCOL) => FailureReason::Protocol,
            _ => FailureReason::Closed,
        },
        Some(ConnectionError::TimedOut) => FailureReason::Timeout,
        _ => FailureReason::Closed,
    }
}

/// Whether the address set differs enough to warrant a `MyTicketChanged`.
fn addr_set_changed(old: &EndpointAddr, new: &EndpointAddr) -> bool {
    old != new
}

/// Emits `MyTicketChanged` when the endpoint's addresses change, at most once
/// per debounce period.
async fn watch_ticket(inner: Arc<Inner>) {
    let mut watcher = inner.endpoint.watch_addr();
    let mut last = watcher.get();
    let closed = inner.endpoint.closed();
    tokio::pin!(closed);
    loop {
        tokio::select! {
            updated = watcher.updated() => {
                if updated.is_err() {
                    break;
                }
            }
            _ = &mut closed => break,
        }
        let current = watcher.get();
        if addr_set_changed(&last, &current) {
            last = current;
            inner.emit(NetEvent::MyTicketChanged);
            tokio::select! {
                _ = sleep(TICKET_CHANGE_DEBOUNCE) => {}
                _ = &mut closed => break,
            }
        }
    }
}

/// Feeds LAN observations through the registry and emits the visible list,
/// at most once per [`LAN_EVENT_DEBOUNCE`].
async fn lan_loop(inner: Arc<Inner>, mut events: tokio::sync::mpsc::Receiver<LanEvent>) {
    let closed = inner.endpoint.closed();
    tokio::pin!(closed);
    let mut dirty = false;
    let mut next_emit = tokio::time::Instant::now();
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                let changed = inner
                    .lan_peers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .apply(ev);
                dirty |= changed;
            }
            _ = tokio::time::sleep_until(next_emit), if dirty => {
                let snapshot = inner
                    .lan_peers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .snapshot();
                inner.emit(NetEvent::LanPeersChanged(snapshot));
                dirty = false;
                next_emit = tokio::time::Instant::now() + LAN_EVENT_DEBOUNCE;
            }
            _ = &mut closed => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(seed: u8) -> EndpointAddr {
        EndpointAddr::new(SecretKey::from_bytes(&[seed; 32]).public())
    }

    #[test]
    fn addr_change_detection_compares_the_address_set() {
        let a = addr(1);
        assert!(!addr_set_changed(&a, &a.clone()));
        let with_ip = a.clone().with_ip_addr("127.0.0.1:9".parse().unwrap());
        assert!(addr_set_changed(&a, &with_ip));
        assert!(addr_set_changed(&with_ip, &a));
    }
}
