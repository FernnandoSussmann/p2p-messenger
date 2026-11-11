//! LAN discovery and presence, driven through the real `Node` over the
//! in-process `MemoryLanBus`. Only the multicast socket is replaced. The
//! package name `core` shadows std `core`, which breaks `#[tokio::test]`, so
//! tests run through `run`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use core::discovery::testing::{
    next_event, restart, spawn_test_node_with, spawn_test_node_wrapped, MemoryLanBus,
    RecordingAddressLookup, SettingsWrapper, TestNode, TestOptions,
};
use core::discovery::{
    event_json, ConnectionState, Direction, FileKnownPeers, LanEvent, LanMode, LanPeer, LanPeerDto,
    NetEvent, NodeConfig, PresenceError, MAX_LAN_PEERS,
};
use core::identity::{
    AccountId, DisplayName, FileSettingsStore, MemorySettingsStore, NodeId, SettingsError,
    SettingsStore, Ticket,
};
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

async fn lan_node(bus: &MemoryLanBus, name: &str) -> TestNode {
    spawn_test_node_with(name, TestOptions::default().with_lan(bus)).await
}

fn names(peers: &[LanPeer]) -> Vec<Option<String>> {
    peers
        .iter()
        .map(|p| p.display_name.as_ref().map(|n| n.as_str().to_owned()))
        .collect()
}

fn has(peers: &[LanPeer], id: &NodeId) -> bool {
    peers.iter().any(|p| p.node_id == *id)
}

fn name_of(peers: &[LanPeer], id: &NodeId) -> Option<String> {
    peers
        .iter()
        .find(|p| p.node_id == *id)
        .and_then(|p| p.display_name.as_ref().map(|n| n.as_str().to_owned()))
}

/// Waits until the node's LAN list satisfies `pred`, checking the current
/// list first and then every `LanPeersChanged` snapshot.
async fn until_lan(
    node: &TestNode,
    rx: &mut Receiver<NetEvent>,
    pred: impl Fn(&[LanPeer]) -> bool,
) -> Vec<LanPeer> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let current = node.node.lan_peers();
        if pred(&current) {
            return current;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(rx, left).await {
            Some(_) => {}
            None => panic!("timed out waiting for the LAN list; last: {current:?}"),
        }
    }
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

fn is_incoming(ev: &NetEvent) -> bool {
    matches!(ev, NetEvent::IncomingRequest(_))
}

fn connected_to(peer: NodeId, direction: Direction) -> impl Fn(&NetEvent) -> bool {
    move |ev| {
        matches!(ev, NetEvent::ConnectionState { node_id, direction: d, state: ConnectionState::Connected, .. }
            if *node_id == peer && *d == direction)
    }
}

fn lan_ticket(node: &TestNode, of: &NodeId) -> String {
    let peer = node
        .node
        .lan_peers()
        .into_iter()
        .find(|p| p.node_id == *of)
        .expect("peer on the LAN");
    LanPeerDto::from(&peer).ticket
}

/// Names and connections live in memory; presence and relay go to `dir`.
struct FilePresence {
    inner: Arc<MemorySettingsStore>,
    file: FileSettingsStore,
}

impl SettingsStore for FilePresence {
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
        self.file.get_presence(id)
    }
    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        self.file.set_presence(id, on)
    }
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        self.file.get_relay_url(id)
    }
    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        self.file.set_relay_url(id, url)
    }
}

fn file_presence(dir: &Path) -> SettingsWrapper {
    let dir = dir.to_path_buf();
    Arc::new(move |inner| {
        Arc::new(FilePresence {
            inner,
            file: FileSettingsStore::new(&dir),
        })
    })
}

/// Fails `set_presence` while `fail` is set.
struct FailingPresence {
    inner: Arc<MemorySettingsStore>,
    fail: Arc<AtomicBool>,
}

impl SettingsStore for FailingPresence {
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
        self.inner.get_presence(id)
    }
    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(SettingsError::Unavailable);
        }
        self.inner.set_presence(id, on)
    }
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        self.inner.get_relay_url(id)
    }
    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        self.inner.set_relay_url(id, url)
    }
}

#[test]
fn lan_sim_b_discovers_a_with_node_id_and_unverified_name() {
    run(async {
        let bus = MemoryLanBus::new();
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        let a = lan_node(&bus, "Alice").await;

        let ev = wait_for(
            &mut eb,
            |ev| matches!(ev, NetEvent::LanPeersChanged(p) if has(p, &a.id())),
        )
        .await;
        let NetEvent::LanPeersChanged(peers) = &ev else {
            unreachable!()
        };
        assert_eq!(name_of(peers, &a.id()).as_deref(), Some("Alice"));
        assert_eq!(event_json(&ev).0, "lan_peers_changed");
        assert_eq!(names(&b.node.lan_peers()), vec![Some("Alice".to_owned())]);
        assert!(!has(&a.node.lan_peers(), &a.id()), "never lists itself");
        let mut ea = a.node.subscribe();
        until_lan(&a, &mut ea, |p| has(p, &b.id())).await; // A browses B too
    });
}

#[test]
fn presence_off_hides_a_and_on_brings_it_back() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = lan_node(&bus, "Alice").await;
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        until_lan(&b, &mut eb, |p| has(p, &a.id())).await;

        a.node.set_presence(false).unwrap();
        assert!(!a.node.presence());
        until_lan(&b, &mut eb, |p| !has(p, &a.id())).await;

        a.node.set_presence(true).unwrap();
        assert!(a.node.presence());
        let peers = until_lan(&b, &mut eb, |p| has(p, &a.id())).await;
        assert_eq!(name_of(&peers, &a.id()).as_deref(), Some("Alice"));
    });
}

#[test]
fn presence_off_still_browses_and_stays_dialable() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = lan_node(&bus, "Alice").await;
        let b = lan_node(&bus, "Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        a.node.set_presence(false).unwrap();
        until_lan(&a, &mut ea, |p| has(p, &b.id())).await;
        until_lan(&b, &mut eb, |p| !has(p, &a.id())).await;

        b.node
            .connect_ticket(&a.node.ticket().unwrap().to_string())
            .unwrap();
        let NetEvent::IncomingRequest(req) = wait_for(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert_eq!(req.node_id, b.id());
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(&mut eb, connected_to(a.id(), Direction::Outbound)).await;
    });
}

#[test]
fn presence_defaults_on_and_persists_across_restart() {
    run(async {
        let store_dir = tempfile::tempdir().unwrap();
        let bus = MemoryLanBus::new();
        let options = TestOptions::default().with_lan(&bus);
        let a =
            spawn_test_node_wrapped("Alice", options, Some(file_presence(store_dir.path()))).await;
        let id = a.id();
        assert!(a.node.presence(), "a fresh account defaults to ON");
        assert_eq!(bus.advertise_count(&id), 1);
        assert!(bus.advertised_record(&id).is_some());

        a.node.set_presence(false).unwrap();
        assert!(bus.advertised_record(&id).is_none());

        let a = restart(a).await;
        assert!(!a.node.presence(), "OFF survives the restart");
        assert_eq!(
            bus.advertise_count(&id),
            1,
            "startup with presence OFF never advertised"
        );
        assert!(bus.advertised_record(&id).is_none());

        a.node.set_presence(true).unwrap();
        let a = restart(a).await;
        assert!(a.node.presence());
        assert!(bus.advertised_record(&id).is_some());
    });
}

#[test]
fn presence_set_persist_failure_leaves_advertising_unchanged() {
    run(async {
        let bus = MemoryLanBus::new();
        let fail = Arc::new(AtomicBool::new(false));
        let wrapper: SettingsWrapper = {
            let fail = fail.clone();
            Arc::new(move |inner| {
                Arc::new(FailingPresence {
                    inner,
                    fail: fail.clone(),
                })
            })
        };
        let a = spawn_test_node_wrapped(
            "Alice",
            TestOptions::default().with_lan(&bus),
            Some(wrapper),
        )
        .await;
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        until_lan(&b, &mut eb, |p| has(p, &a.id())).await;

        fail.store(true, Ordering::SeqCst);
        let err = a.node.set_presence(false).unwrap_err();
        assert!(matches!(err, PresenceError::Settings(_)));
        assert!(a.node.presence());
        assert!(bus.advertised_record(&a.id()).is_some());
        tokio::time::sleep(QUIET).await;
        assert!(has(&b.node.lan_peers(), &a.id()), "still visible");

        fail.store(false, Ordering::SeqCst);
        a.node.set_presence(false).unwrap();
        until_lan(&b, &mut eb, |p| !has(p, &a.id())).await;

        fail.store(true, Ordering::SeqCst);
        assert!(a.node.set_presence(true).is_err());
        assert!(!a.node.presence());
        assert!(bus.advertised_record(&a.id()).is_none());
    });
}

#[test]
fn lan_pick_uses_the_same_trust_gate() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = lan_node(&bus, "Alice").await;
        let b = lan_node(&bus, "Bob").await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());
        until_lan(&b, &mut eb, |p| has(p, &a.id())).await;

        let ticket = lan_ticket(&b, &a.id());
        assert_eq!(Ticket::parse(&ticket).unwrap().node_id, a.id());
        b.node.connect_ticket(&ticket).unwrap();
        wait_for(&mut ea, is_incoming).await;
        assert_eq!(count_incoming_for(&mut ea, QUIET).await, 0, "one prompt");
        let pending = a.node.pending_incoming();
        assert_eq!(pending.len(), 1, "exactly one prompt on A");
        assert_eq!(pending[0].node_id, b.id());
        a.node.respond_incoming(&b.id(), true).unwrap();
        wait_for(&mut eb, connected_to(a.id(), Direction::Outbound)).await;

        b.node.connect_ticket(&lan_ticket(&b, &a.id())).unwrap();
        wait_for(&mut eb, connected_to(a.id(), Direction::Outbound)).await;
        assert!(
            a.node.pending_incoming().is_empty(),
            "a known peer reconnects silently"
        );
    });
}

#[test]
fn same_name_two_node_ids_stay_distinct() {
    run(async {
        let bus = MemoryLanBus::new();
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        let a1 = lan_node(&bus, "Twin").await;
        let a2 = lan_node(&bus, "Twin").await;

        let peers = until_lan(&b, &mut eb, |p| has(p, &a1.id()) && has(p, &a2.id())).await;
        assert_eq!(peers.len(), 2);
        assert_ne!(peers[0].node_id, peers[1].node_id);
        assert_eq!(
            names(&peers),
            vec![Some("Twin".into()), Some("Twin".into())]
        );
    });
}

#[test]
fn malformed_and_oversized_mdns_records_never_panic() {
    run(async {
        let bus = MemoryLanBus::new();
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        let victim = b.id();
        let addr_for =
            |id: NodeId| iroh::EndpointAddr::new(id).with_ip_addr(([10, 9, 8, 7], 4000).into());
        let key = || iroh::SecretKey::generate().public();
        let (huge, bad_utf8, bidi) = (key(), key(), key());

        bus.inject(
            &victim,
            LanEvent::Discovered {
                addr: addr_for(huge),
                user_data: Some(vec![b'a'; 64 * 1024]),
            },
        )
        .await;
        bus.inject(
            &victim,
            LanEvent::Discovered {
                addr: addr_for(bad_utf8),
                user_data: Some(vec![0xff, 0xfe, 0xfd]),
            },
        )
        .await;
        bus.inject(
            &victim,
            LanEvent::Discovered {
                addr: addr_for(bidi),
                user_data: Some("Eve\u{202E}live\n".as_bytes().to_vec()),
            },
        )
        .await;
        bus.inject(
            &victim,
            LanEvent::Discovered {
                addr: addr_for(victim),
                user_data: Some(b"Myself".to_vec()),
            },
        )
        .await;
        bus.inject(
            &victim,
            LanEvent::Discovered {
                addr: iroh::EndpointAddr::new(key()),
                user_data: None,
            },
        )
        .await;
        for _ in 0..1000 {
            bus.inject(
                &victim,
                LanEvent::Discovered {
                    addr: addr_for(key()),
                    user_data: None,
                },
            )
            .await;
        }

        let peers = until_lan(&b, &mut eb, |p| p.len() == MAX_LAN_PEERS).await;
        tokio::time::sleep(QUIET).await;
        let peers = if peers.len() == b.node.lan_peers().len() {
            peers
        } else {
            b.node.lan_peers()
        };
        assert!(peers.len() <= MAX_LAN_PEERS);
        assert!(!has(&peers, &victim), "own id is never listed");
        for bad in [huge, bad_utf8, bidi] {
            assert!(
                !has(&peers, &bad) || name_of(&peers, &bad).is_none(),
                "bad names decode to None"
            );
        }
        assert!(peers.iter().all(|p| !p.addr.addrs.is_empty()));
    });
}

#[test]
fn lan_peers_are_never_persisted() {
    run(async {
        let known_dir = tempfile::tempdir().unwrap();
        let settings_dir = tempfile::tempdir().unwrap();
        let bus = MemoryLanBus::new();
        let known = Arc::new(FileKnownPeers::new(known_dir.path(), AccountId::new()));
        let a = spawn_test_node_wrapped(
            "Alice",
            TestOptions::default().with_lan(&bus).with_known(known),
            Some(file_presence(settings_dir.path())),
        )
        .await;
        let b = lan_node(&bus, "Bob").await;
        let mut ea = a.node.subscribe();
        until_lan(&a, &mut ea, |p| has(p, &b.id())).await;
        // Force a settings write so the check below has a file to read.
        a.node.set_presence(true).unwrap();
        a.node.refresh_local_name();

        let needle = b.id().to_string();
        for dir in [known_dir.path(), settings_dir.path(), a.dir.path()] {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                assert!(!text.contains(&needle), "{path:?} holds a LAN peer");
            }
        }
        assert!(settings_dir.path().join("settings.json").exists());
    });
}

#[test]
fn internet_published_record_has_no_name() {
    run(async {
        let bus = MemoryLanBus::new();
        let recorder = RecordingAddressLookup::new();
        let canary = "CanaryName7f3a";
        let a = spawn_test_node_with(
            canary,
            TestOptions::default()
                .with_lan(&bus)
                .with_recorder(&recorder),
        )
        .await;

        let deadline = tokio::time::Instant::now() + WAIT;
        while recorder.published().is_empty() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the endpoint never published"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(bus.advertised_record(&a.id()).is_some());

        a.node.set_presence(false).unwrap();
        a.node.set_presence(true).unwrap();
        a.identity.set_display_name("CanaryRenamed9c1").unwrap();
        a.node.refresh_local_name();
        tokio::time::sleep(QUIET).await;

        let published = recorder.published();
        assert!(!published.is_empty());
        for data in &published {
            assert!(
                data.user_data().is_none(),
                "endpoint-wide data must carry no name: {data:?}"
            );
            let dump = format!("{data:?}");
            assert!(!dump.contains("Canary"), "{dump}");
        }
    });
}

#[test]
fn presence_off_withdraws_name_from_mdns_record() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = lan_node(&bus, "Alice").await;
        assert_eq!(
            bus.advertised_record(&a.id()),
            Some(Some(b"Alice".to_vec()))
        );

        a.node.set_presence(false).unwrap();
        assert_eq!(bus.advertised_record(&a.id()), None, "no record, no name");

        a.node.set_presence(true).unwrap();
        a.identity.set_display_name("Alicia").unwrap();
        a.node.refresh_local_name();
        assert_eq!(
            bus.advertised_record(&a.id()),
            Some(Some(b"Alicia".to_vec())),
            "ON re-attaches the current name"
        );
    });
}

#[test]
fn refresh_local_name_republishes_after_rename() {
    run(async {
        let bus = MemoryLanBus::new();
        let a = lan_node(&bus, "Alice").await;
        let b = lan_node(&bus, "Bob").await;
        let mut eb = b.node.subscribe();
        until_lan(&b, &mut eb, |p| {
            name_of(p, &a.id()).as_deref() == Some("Alice")
        })
        .await;

        a.identity.set_display_name("Alicia").unwrap();
        a.node.refresh_local_name();
        until_lan(&b, &mut eb, |p| {
            name_of(p, &a.id()).as_deref() == Some("Alicia")
        })
        .await;

        a.node.set_presence(false).unwrap();
        until_lan(&b, &mut eb, |p| !has(p, &a.id())).await;
        let before = bus.advertise_count(&a.id());
        a.identity.set_display_name("Alice Off").unwrap();
        a.node.refresh_local_name();
        tokio::time::sleep(QUIET).await;
        assert_eq!(bus.advertise_count(&a.id()), before, "nothing published");
        assert!(bus.advertised_record(&a.id()).is_none());
        assert!(!has(&b.node.lan_peers(), &a.id()));
    });
}

#[test]
fn lan_events_are_debounced() {
    run(async {
        let bus = MemoryLanBus::new();
        let c = lan_node(&bus, "Carol").await;
        let mut ec = c.node.subscribe();
        let t0 = tokio::time::Instant::now();
        for i in 0..40u8 {
            let id = iroh::SecretKey::from_bytes(&[i + 100; 32]).public();
            bus.inject(
                &c.id(),
                LanEvent::Discovered {
                    addr: iroh::EndpointAddr::new(id).with_ip_addr(([10, 1, 0, i], 1).into()),
                    user_data: None,
                },
            )
            .await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let injected_for = t0.elapsed();
        let mut count = 0usize;
        while next_event(&mut ec, Duration::from_millis(500))
            .await
            .is_some()
        {
            count += 1;
        }
        let max = (injected_for.as_millis() / 250) as usize + 3;
        assert!(
            (1..=max).contains(&count),
            "{count} events in {injected_for:?} (max {max})"
        );
        assert_eq!(c.node.lan_peers().len(), 40);
    });
}

#[test]
#[ignore = "needs a multicast-capable interface; run: cargo test -p core --test discovery_lan -- --ignored"]
fn real_mdns_two_nodes_discover_each_other() {
    run(async {
        let options = || {
            let mut options = TestOptions::default();
            options.config = NodeConfig::loopback().with_lan(LanMode::Mdns);
            options
        };
        let a = spawn_test_node_with("Alice", options()).await;
        let b = spawn_test_node_with("Bob", options()).await;
        let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

        let from_b = until_lan(&b, &mut eb, |p| has(p, &a.id())).await;
        assert_eq!(name_of(&from_b, &a.id()).as_deref(), Some("Alice"));
        until_lan(&a, &mut ea, |p| has(p, &b.id())).await;

        a.node.set_presence(false).unwrap();
        until_lan(&b, &mut eb, |p| !has(p, &a.id())).await;
        a.node.set_presence(true).unwrap();
        until_lan(&b, &mut eb, |p| has(p, &a.id())).await;
    });
}
