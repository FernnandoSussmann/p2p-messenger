//! Test doubles for the discovery slice. Everything here drives the same
//! [`Node`] code paths as production; there is no alternate gate.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use iroh::address_lookup::{
    AddressLookupBuilder, AddressLookupBuilderError, EndpointInfo, MemoryLookup,
};
use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, Watcher};
use protocol::ALPN;
use tempfile::TempDir;
use tokio::sync::broadcast;

use super::events::NetEvent;
use super::flood::GateLimits;
use super::hello::encode_hello;
use super::known_peers::{KnownPeers, KnownPeersError};
use super::node::{Node, NodeConfig, NodeDeps};
use super::ticket_provider::LiveTicketProvider;
use crate::identity::testing::FakeKeychain;
use crate::identity::{DisplayName, IdentityService, MemorySettingsStore, NodeId, SettingsStore};

/// In-memory [`KnownPeers`]. Names are stored as advisory values only.
#[derive(Default)]
pub struct MemoryKnownPeers {
    peers: Mutex<HashMap<NodeId, Option<DisplayName>>>,
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

    /// The last-seen name stored for `id`, if the peer is known and named.
    pub fn name_of(&self, id: &NodeId) -> Option<String> {
        self.lock()
            .get(id)
            .and_then(|n| n.as_ref().map(|n| n.as_str().to_owned()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<NodeId, Option<DisplayName>>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl KnownPeers for MemoryKnownPeers {
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError> {
        Ok(self.lock().contains_key(id))
    }

    fn add(&self, id: NodeId, display_name: Option<DisplayName>) -> Result<(), KnownPeersError> {
        self.lock().insert(id, display_name);
        Ok(())
    }
}

/// A store whose every operation fails, to prove the gate fails closed.
pub struct FailingKnownPeers;

impl KnownPeers for FailingKnownPeers {
    fn is_known(&self, _id: &NodeId) -> Result<bool, KnownPeersError> {
        Err(KnownPeersError::Corrupt)
    }

    fn add(&self, _id: NodeId, _name: Option<DisplayName>) -> Result<(), KnownPeersError> {
        Err(KnownPeersError::Corrupt)
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

/// How a test node is configured beyond the defaults.
#[derive(Clone)]
pub struct TestOptions {
    pub config: NodeConfig,
    pub known: Arc<dyn KnownPeers>,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            config: NodeConfig::loopback(),
            known: Arc::new(MemoryKnownPeers::new()),
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
    start(identity, dir, keychain, settings, tickets, options).await
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
        ..
    } = old;
    node.shutdown().await;
    start(identity, dir, keychain, settings, tickets, options).await
}

async fn start(
    identity: TestIdentity,
    dir: TempDir,
    keychain: Arc<FakeKeychain>,
    settings: Arc<MemorySettingsStore>,
    tickets: LiveTicketProvider,
    options: TestOptions,
) -> TestNode {
    let settings_dyn: Arc<dyn SettingsStore + Send + Sync> = settings.clone();
    let node = Node::start(
        &identity,
        NodeDeps {
            settings: settings_dyn,
            known: options.known.clone(),
            tickets: tickets.clone(),
        },
        options.config.clone(),
    )
    .await
    .expect("node start");
    TestNode {
        node,
        identity,
        known: options.known.clone(),
        dir,
        keychain,
        settings,
        tickets,
        options,
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
    let (mut send, _recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    // The acceptor may close mid-write on oversized input; that is the test.
    let _ = send.write_all(first_frame).await;
    let _ = send.finish();
    Ok(RawDial {
        conn,
        _endpoint: endpoint,
    })
}
