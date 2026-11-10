//! LAN discovery: who is on this network, and whether we advertise ourselves.
//!
//! All interpretation of what arrives from the network lives in the pure
//! [`encode_lan_name`], [`decode_lan_name`] and [`LanPeers`], which run in the
//! mandatory test gate. [`MdnsLan`] only moves bytes between the multicast
//! socket and [`LanEvent`]s.
//!
//! # Transport choice: `iroh-mdns-address-lookup` 0.6 (swarm-discovery)
//!
//! `iroh-mdns-peer-lookup` was not chosen: it exposes `Options::announce` but
//! no discovered/expired subscription that fits [`LanEvent`].
//!
//! - (a) Advertising is a build-time flag (`advertise(bool)`), with no runtime
//!   toggle. [`MdnsLan`] therefore keeps a permanent browse-only instance
//!   (`advertise(false)`) and builds a separate advertising instance only while
//!   presence is ON. Turning presence OFF publishes empty data (swarm-discovery
//!   then announces the peer as expired, so receivers drop it) and drops the
//!   instance after a short grace period, which removes the record and its TXT
//!   attributes for good.
//! - (b) The only per-record payload is the TXT `user-data` attribute, fed from
//!   `EndpointData::user_data` of whatever is passed to `AddressLookup::publish`
//!   on that mDNS instance (at most `UserData::MAX_LENGTH` = 245 bytes of
//!   UTF-8). The advertising instance is never registered with the endpoint;
//!   we call `publish` on it ourselves with the endpoint's addresses plus the
//!   name. The name therefore reaches the mDNS instance only.
//! - (c) `DiscoveryEvent::Discovered { endpoint_info, .. }` and
//!   `DiscoveryEvent::Expired { endpoint_id }` (non-exhaustive; unknown
//!   variants are skipped).
//! - (d) With the endpoint-wide `set_user_data_for_address_lookup`, the same
//!   `EndpointData` would reach every registered lookup, including the
//!   internet publishers of the N0 preset. This module never calls it, so the
//!   endpoint-wide data carries no name; a test asserts this.
//!
//! The mDNS service name is `p2pmessenger`, so other iroh applications on the
//! LAN (default service `irohv1`) do not appear in the peer list.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use iroh::address_lookup::{AddressLookup, EndpointData, UserData};
use iroh::{Endpoint, EndpointAddr, Watcher};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::identity::{validate_display_name, DisplayName, NodeId};

/// Most peers the registry holds; further discoveries are ignored until
/// entries expire.
pub const MAX_LAN_PEERS: usize = 256;

/// Largest name payload carried in an mDNS record.
pub const LAN_NAME_MAX_BYTES: usize = UserData::MAX_LENGTH;

const SERVICE_NAME: &str = "p2pmessenger";
const EVENT_BUFFER: usize = 1024;
const WITHDRAW_GRACE: Duration = Duration::from_secs(1);

/// The name as record bytes, cut on a `char` boundary to at most `max_bytes`.
pub fn encode_lan_name(name: &DisplayName, max_bytes: usize) -> Vec<u8> {
    let s = name.as_str();
    let mut end = s.len().min(max_bytes);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.as_bytes()[..end].to_vec()
}

/// The self-asserted name in a record, or `None` if it is oversize, not UTF-8
/// or fails display-name validation.
pub fn decode_lan_name(raw: &[u8], max_bytes: usize) -> Option<DisplayName> {
    if raw.len() > max_bytes {
        return None;
    }
    validate_display_name(std::str::from_utf8(raw).ok()?).ok()
}

/// A peer seen on the LAN. The name is self-asserted and UNVERIFIED.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPeer {
    pub node_id: NodeId,
    pub display_name: Option<DisplayName>,
    pub addr: EndpointAddr,
}

/// A raw observation from the LAN transport; `user_data` is uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanEvent {
    Discovered {
        addr: EndpointAddr,
        user_data: Option<Vec<u8>>,
    },
    Expired(NodeId),
}

/// The only place [`LanEvent`]s become peers: keyed by NodeId, own id
/// filtered, capped at [`MAX_LAN_PEERS`].
pub struct LanPeers {
    own_id: NodeId,
    peers: BTreeMap<NodeId, LanPeer>,
}

impl LanPeers {
    pub fn new(own_id: NodeId) -> Self {
        Self {
            own_id,
            peers: BTreeMap::new(),
        }
    }

    /// True if the visible list changed.
    pub fn apply(&mut self, ev: LanEvent) -> bool {
        match ev {
            LanEvent::Expired(id) => self.peers.remove(&id).is_some(),
            LanEvent::Discovered { addr, user_data } => {
                let node_id = addr.id;
                if node_id == self.own_id || addr.addrs.is_empty() {
                    return false;
                }
                if !self.peers.contains_key(&node_id) && self.peers.len() >= MAX_LAN_PEERS {
                    return false;
                }
                let display_name = user_data
                    .as_deref()
                    .and_then(|raw| decode_lan_name(raw, LAN_NAME_MAX_BYTES));
                let peer = LanPeer {
                    node_id,
                    display_name,
                    addr,
                };
                self.peers.insert(node_id, peer.clone()) != Some(peer)
            }
        }
    }

    /// Sorted by NodeId.
    pub fn snapshot(&self) -> Vec<LanPeer> {
        self.peers.values().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanError {
    Transport(String),
}

impl fmt::Display for LanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "LAN discovery failed: {e}"),
        }
    }
}

impl std::error::Error for LanError {}

/// The transport seam. [`MdnsLan`] ships; `testing::MemoryLan` is a
/// deterministic in-process bus.
pub trait LanDiscovery: Send + Sync + 'static {
    /// Starts (`on`, with the name to attach to the record only) or withdraws
    /// (`!on`) our LAN record. Browsing is unaffected.
    fn set_advertise(&self, on: bool, local_name: Option<&DisplayName>) -> Result<(), LanError>;
    /// The stream of observations. Called once by the node.
    fn events(&self) -> mpsc::Receiver<LanEvent>;
}

/// Which LAN transport a node uses.
#[derive(Clone)]
pub enum LanMode {
    Mdns,
    Off,
    #[cfg(any(test, feature = "test-support"))]
    Memory(super::testing::MemoryLanBus),
}

/// No LAN at all: advertising is a no-op and nothing is ever discovered.
pub(super) struct NoLan;

impl LanDiscovery for NoLan {
    fn set_advertise(&self, _on: bool, _local_name: Option<&DisplayName>) -> Result<(), LanError> {
        Ok(())
    }

    fn events(&self) -> mpsc::Receiver<LanEvent> {
        mpsc::channel(1).1
    }
}

struct Advertiser {
    lookup: MdnsAddressLookup,
    task: JoinHandle<()>,
}

impl Advertiser {
    fn withdraw(self, runtime: &tokio::runtime::Handle) {
        self.task.abort();
        self.lookup.publish(&EndpointData::default());
        let lookup = self.lookup.clone();
        runtime.spawn(async move {
            tokio::time::sleep(WITHDRAW_GRACE).await;
            drop(lookup);
        });
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The shipping LAN transport, over multicast DNS.
pub struct MdnsLan {
    endpoint: Endpoint,
    runtime: tokio::runtime::Handle,
    _browser: MdnsAddressLookup,
    browse_task: JoinHandle<()>,
    events: Mutex<Option<mpsc::Receiver<LanEvent>>>,
    advertiser: Mutex<Option<Advertiser>>,
}

fn transport_err(e: impl fmt::Display) -> LanError {
    LanError::Transport(e.to_string())
}

/// What we publish into our own mDNS record: the endpoint's addresses and,
/// only here, the name.
fn advertised_data(addr: &EndpointAddr, user_data: Option<UserData>) -> EndpointData {
    let mut data = EndpointData::from(addr.clone());
    data.set_user_data(user_data);
    data
}

fn name_user_data(name: &DisplayName) -> Option<UserData> {
    String::from_utf8(encode_lan_name(name, LAN_NAME_MAX_BYTES))
        .ok()?
        .parse()
        .ok()
}

fn to_lan_event(ev: DiscoveryEvent) -> Option<LanEvent> {
    match ev {
        DiscoveryEvent::Discovered { endpoint_info, .. } => {
            let user_data = endpoint_info
                .data
                .user_data()
                .map(|u| u.as_ref().as_bytes().to_vec());
            Some(LanEvent::Discovered {
                addr: endpoint_info.into_endpoint_addr(),
                user_data,
            })
        }
        DiscoveryEvent::Expired { endpoint_id } => Some(LanEvent::Expired(endpoint_id)),
        _ => None,
    }
}

impl MdnsLan {
    /// Starts browsing. Must run inside a tokio runtime. Nothing is
    /// advertised until [`LanDiscovery::set_advertise`] is called with `true`.
    pub async fn start(endpoint: Endpoint) -> Result<Self, LanError> {
        let browser = MdnsAddressLookup::builder()
            .service_name(SERVICE_NAME)
            .advertise(false)
            .build(endpoint.id())
            .map_err(transport_err)?;
        let mut stream = browser.subscribe().await;
        let (tx, rx) = mpsc::channel(EVENT_BUFFER);
        let browse_task = tokio::spawn(async move {
            while let Some(ev) = stream.next().await {
                let Some(ev) = to_lan_event(ev) else {
                    continue;
                };
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            endpoint,
            runtime: tokio::runtime::Handle::current(),
            _browser: browser,
            browse_task,
            events: Mutex::new(Some(rx)),
            advertiser: Mutex::new(None),
        })
    }

    fn spawn_publisher(
        &self,
        lookup: MdnsAddressLookup,
        user_data: Option<UserData>,
    ) -> JoinHandle<()> {
        let endpoint = self.endpoint.clone();
        self.runtime.spawn(async move {
            let mut watcher = endpoint.watch_addr();
            loop {
                lookup.publish(&advertised_data(&watcher.get(), user_data.clone()));
                if watcher.updated().await.is_err() {
                    break;
                }
            }
        })
    }
}

impl LanDiscovery for MdnsLan {
    fn set_advertise(&self, on: bool, local_name: Option<&DisplayName>) -> Result<(), LanError> {
        let mut slot = self
            .advertiser
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !on {
            if let Some(old) = slot.take() {
                old.withdraw(&self.runtime);
            }
            return Ok(());
        }
        let user_data = local_name.and_then(name_user_data);
        // A record cannot drop a stale name in place, so it is only reused
        // when a new name replaces the old one.
        if let (Some(current), Some(_)) = (slot.as_mut(), &user_data) {
            current.task.abort();
            current.task = self.spawn_publisher(current.lookup.clone(), user_data);
            return Ok(());
        }
        if let Some(old) = slot.take() {
            old.withdraw(&self.runtime);
        }
        let _guard = self.runtime.enter();
        let lookup = MdnsAddressLookup::builder()
            .service_name(SERVICE_NAME)
            .advertise(true)
            .build(self.endpoint.id())
            .map_err(transport_err)?;
        let task = self.spawn_publisher(lookup.clone(), user_data);
        *slot = Some(Advertiser { lookup, task });
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

impl Drop for MdnsLan {
    fn drop(&mut self) {
        self.browse_task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn id(seed: u8) -> NodeId {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn addr(seed: u8) -> EndpointAddr {
        EndpointAddr::new(id(seed)).with_ip_addr(([10, 0, 0, seed], 4000).into())
    }

    fn name(raw: &str) -> DisplayName {
        validate_display_name(raw).unwrap()
    }

    fn found(seed: u8, user_data: Option<&[u8]>) -> LanEvent {
        LanEvent::Discovered {
            addr: addr(seed),
            user_data: user_data.map(<[u8]>::to_vec),
        }
    }

    #[test]
    fn encode_truncates_on_a_char_boundary() {
        let n = name("aé日本");
        assert_eq!(encode_lan_name(&n, 100), "aé日本".as_bytes());
        // 'a' is 1 byte, 'é' 2, '日' 3: a cut inside '日' falls back to "aé".
        assert_eq!(encode_lan_name(&n, 4), "aé".as_bytes());
        assert_eq!(encode_lan_name(&n, 6), "aé日".as_bytes());
        assert_eq!(encode_lan_name(&n, 0), b"");
        for max in 0..12 {
            assert!(std::str::from_utf8(&encode_lan_name(&n, max)).is_ok());
        }
    }

    #[test]
    fn decode_round_trips_and_rejects_bad_input() {
        let n = name("Alice");
        assert_eq!(decode_lan_name(&encode_lan_name(&n, 245), 245), Some(n));
        assert_eq!(decode_lan_name(&[0xff, 0xfe], 245), None);
        assert_eq!(decode_lan_name(&vec![b'a'; 246], 245), None);
        assert_eq!(decode_lan_name(b"Eve\xE2\x80\xAEx", 245), None);
        assert_eq!(decode_lan_name(b"a\nb", 245), None);
        assert_eq!(decode_lan_name(b"   ", 245), None);
        assert_eq!(decode_lan_name(b"", 245), None);
    }

    #[test]
    fn discovered_peer_is_listed_with_its_decoded_name() {
        let mut peers = LanPeers::new(id(1));
        assert!(peers.apply(found(2, Some(b"Alice"))));
        let snap = peers.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].node_id, id(2));
        assert_eq!(snap[0].display_name, Some(name("Alice")));
        assert_eq!(snap[0].addr, addr(2));
    }

    #[test]
    fn own_node_id_is_filtered() {
        let mut peers = LanPeers::new(id(1));
        assert!(!peers.apply(found(1, Some(b"Me"))));
        assert!(peers.snapshot().is_empty());
    }

    #[test]
    fn empty_address_is_ignored() {
        let mut peers = LanPeers::new(id(1));
        let ev = LanEvent::Discovered {
            addr: EndpointAddr::new(id(2)),
            user_data: None,
        };
        assert!(!peers.apply(ev));
        assert!(peers.snapshot().is_empty());
    }

    #[test]
    fn bad_names_become_none_but_the_peer_stays() {
        let mut peers = LanPeers::new(id(1));
        let huge = vec![b'a'; 64 * 1024];
        for (seed, raw) in [
            (2u8, huge.as_slice()),
            (3, &[0xff, 0xfe][..]),
            (4, "Eve\u{202E}".as_bytes()),
            (5, b"a\x07b"),
        ] {
            assert!(peers.apply(found(seed, Some(raw))));
        }
        assert!(peers.snapshot().iter().all(|p| p.display_name.is_none()));
        assert_eq!(peers.snapshot().len(), 4);
    }

    #[test]
    fn name_update_replaces_without_a_second_entry() {
        let mut peers = LanPeers::new(id(1));
        assert!(peers.apply(found(2, Some(b"Alice"))));
        assert!(!peers.apply(found(2, Some(b"Alice"))));
        assert!(peers.apply(found(2, Some(b"Alicia"))));
        let snap = peers.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].display_name, Some(name("Alicia")));
    }

    #[test]
    fn same_name_distinct_node_ids_stay_distinct() {
        let mut peers = LanPeers::new(id(1));
        assert!(peers.apply(found(2, Some(b"Alice"))));
        assert!(peers.apply(found(3, Some(b"Alice"))));
        assert_eq!(peers.snapshot().len(), 2);
    }

    #[test]
    fn expired_removes_and_unknown_expiry_changes_nothing() {
        let mut peers = LanPeers::new(id(1));
        peers.apply(found(2, None));
        assert!(!peers.apply(LanEvent::Expired(id(9))));
        assert!(peers.apply(LanEvent::Expired(id(2))));
        assert!(peers.snapshot().is_empty());
    }

    #[test]
    fn registry_is_capped_but_known_peers_still_update() {
        let mut peers = LanPeers::new(id(0));
        let mut ids = Vec::new();
        for i in 0..(MAX_LAN_PEERS + 50) {
            let key = SecretKey::from_bytes(&{
                let mut b = [0u8; 32];
                b[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
                b
            });
            let a = EndpointAddr::new(key.public()).with_ip_addr(([10, 0, 0, 1], 1).into());
            ids.push(a.clone());
            peers.apply(LanEvent::Discovered {
                addr: a,
                user_data: None,
            });
        }
        assert_eq!(peers.snapshot().len(), MAX_LAN_PEERS);
        let first = ids[0].clone();
        assert!(peers.apply(LanEvent::Discovered {
            addr: first.clone(),
            user_data: Some(b"Renamed".to_vec()),
        }));
        assert_eq!(peers.snapshot().len(), MAX_LAN_PEERS);
        assert!(peers.apply(LanEvent::Expired(first.id)));
        assert!(peers.apply(LanEvent::Discovered {
            addr: ids[MAX_LAN_PEERS].clone(),
            user_data: None,
        }));
    }

    #[test]
    fn snapshot_is_sorted_by_node_id() {
        let mut peers = LanPeers::new(id(0));
        for seed in [5, 3, 9, 1] {
            peers.apply(found(seed, None));
        }
        let ids: Vec<_> = peers.snapshot().iter().map(|p| p.node_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn advertised_data_carries_the_name_only_in_user_data() {
        let a = addr(2);
        let plain = advertised_data(&a, None);
        assert!(plain.user_data().is_none());
        assert_eq!(plain.ip_addrs().count(), 1);
        let ud = name_user_data(&name("Alice"));
        let named = advertised_data(&a, ud);
        assert_eq!(named.user_data().map(AsRef::as_ref), Some("Alice"));
        assert_eq!(named.ip_addrs().count(), 1);
    }

    #[test]
    fn name_user_data_stays_within_the_txt_limit() {
        let long = name(&"日".repeat(100));
        let ud = name_user_data(&long).unwrap();
        assert!(ud.as_ref().len() <= UserData::MAX_LENGTH);
    }

    #[test]
    fn mdns_events_map_to_raw_lan_events() {
        let id = id(7);
        let ev = to_lan_event(DiscoveryEvent::Expired { endpoint_id: id });
        assert_eq!(ev, Some(LanEvent::Expired(id)));
    }
}
