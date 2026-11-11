//! The network Tauri commands, the managed `NetState` and the event forwarder.
//!
//! The logic lives in plain functions over `NetState` so it can be tested
//! without a Tauri runtime; the `#[tauri::command]` wrappers only forward the
//! managed state. Event names and payloads come from core's `event_json`.

use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use messenger_core::discovery::{
    event_json, ConnectError, FileKnownPeers, IncomingRequestDto, LiveTicketProvider, MyTicketDto,
    NetEvent, Node, NodeConfig, NodeDeps, RespondError, StartError,
};
use messenger_core::identity::{
    ticket_to_qr_svg, FileSettingsStore, IdentityStatus, NodeId, TicketError,
};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::RecvError;

use crate::identity_cmd::AppIdentity;

const TICKET_READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Webview-facing error. Messages never embed backend or secret details.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum NetCmdError {
    NotReady,
    InvalidTicket,
    SelfDial,
    NoSuchRequest,
    Internal,
}

impl From<ConnectError> for NetCmdError {
    fn from(err: ConnectError) -> Self {
        match err {
            ConnectError::InvalidTicket => Self::InvalidTicket,
            ConnectError::SelfDial => Self::SelfDial,
            ConnectError::ShuttingDown => Self::NotReady,
        }
    }
}

impl From<RespondError> for NetCmdError {
    fn from(err: RespondError) -> Self {
        match err {
            RespondError::NoSuchRequest => Self::NoSuchRequest,
        }
    }
}

impl From<TicketError> for NetCmdError {
    fn from(_: TicketError) -> Self {
        Self::Internal
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectDto {
    pub node_id: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Started,
    Closed,
}

/// The managed network state: at most one node per process.
pub struct NetState {
    node: RwLock<Option<Arc<Node>>>,
    phase: tokio::sync::Mutex<Phase>,
}

impl Default for NetState {
    fn default() -> Self {
        Self {
            node: RwLock::new(None),
            phase: tokio::sync::Mutex::new(Phase::Idle),
        }
    }
}

impl NetState {
    pub fn node(&self) -> Result<Arc<Node>, NetCmdError> {
        self.node
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or(NetCmdError::NotReady)
    }

    /// Runs `start` unless a node already runs or the state was shut down.
    /// Returns whether this call stored a new node. A failed start can be
    /// retried by a later call.
    pub async fn ensure_started_with(
        &self,
        start: impl Future<Output = Result<Node, StartError>>,
    ) -> Result<bool, StartError> {
        let mut phase = self.phase.lock().await;
        if *phase != Phase::Idle {
            return Ok(false);
        }
        let node = start.await?;
        if let Ok(mut slot) = self.node.write() {
            *slot = Some(Arc::new(node));
        }
        *phase = Phase::Started;
        Ok(true)
    }

    /// Takes the node out and shuts it down, giving up after `limit`. Later
    /// calls, and later start attempts, are no-ops.
    pub async fn shutdown_with_timeout(&self, limit: Duration) {
        let taken = {
            let mut phase = self.phase.lock().await;
            *phase = Phase::Closed;
            self.node.write().ok().and_then(|mut slot| slot.take())
        };
        if let Some(node) = taken.and_then(|n| Arc::try_unwrap(n).ok()) {
            let _ = tokio::time::timeout(limit, node.shutdown()).await;
        }
    }
}

pub async fn my_ticket(state: &NetState) -> Result<MyTicketDto, NetCmdError> {
    let node = state.node()?;
    let ticket = node.ticket_when_ready(TICKET_READY_TIMEOUT).await?;
    let qr_svg = ticket_to_qr_svg(&ticket)?;
    Ok(MyTicketDto {
        ticket: ticket.to_string(),
        qr_svg,
    })
}

pub fn connect_ticket(state: &NetState, ticket: &str) -> Result<ConnectDto, NetCmdError> {
    let node_id = state.node()?.connect_ticket(ticket)?;
    Ok(ConnectDto {
        node_id: node_id.to_string(),
    })
}

pub fn respond_incoming(state: &NetState, node_id: &str, accept: bool) -> Result<(), NetCmdError> {
    let node = state.node()?;
    let peer = NodeId::from_str(node_id).map_err(|_| NetCmdError::NoSuchRequest)?;
    node.respond_incoming(&peer, accept)?;
    Ok(())
}

pub fn pending_incoming(state: &NetState) -> Result<Vec<IncomingRequestDto>, NetCmdError> {
    Ok(state
        .node()?
        .pending_incoming()
        .iter()
        .map(IncomingRequestDto::from)
        .collect())
}

/// Maps a core event to its external name and payload and hands it to `emit`.
pub fn forward(ev: &NetEvent, emit: impl Fn(&str, Value)) {
    let (name, payload) = event_json(ev);
    emit(name, payload);
}

/// Everything `ensure_started` needs besides the identity service.
pub struct NetBoot {
    pub data_dir: std::path::PathBuf,
    pub settings: Arc<FileSettingsStore>,
    pub tickets: LiveTicketProvider,
}

/// Starts the node in the background, once, if an identity exists. A start
/// failure leaves the app usable: commands report `NotReady`.
pub fn ensure_started(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NetState>();
        let boot = app.state::<NetBoot>();
        let identity = app.state::<AppIdentity>();
        let account = match identity.status() {
            IdentityStatus::Present { account_id, .. } => account_id,
            _ => return,
        };
        let deps = NodeDeps {
            settings: boot.settings.clone(),
            known: Arc::new(FileKnownPeers::new(boot.data_dir.clone(), account)),
            tickets: boot.tickets.clone(),
        };
        let start = Node::start(&*identity, deps, NodeConfig::desktop());
        if !matches!(state.ensure_started_with(start).await, Ok(true)) {
            return;
        }
        let Ok(node) = state.node() else { return };
        let mut events = node.subscribe();
        drop(node);
        let _ = app.emit("my_ticket_changed", Value::Null);
        loop {
            match events.recv().await {
                Ok(ev) => forward(&ev, |name, payload| {
                    let _ = app.emit(name, payload);
                }),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
}

#[tauri::command]
pub async fn net_my_ticket(state: State<'_, NetState>) -> Result<MyTicketDto, NetCmdError> {
    my_ticket(&state).await
}

#[tauri::command]
pub async fn net_connect_ticket(
    state: State<'_, NetState>,
    ticket: String,
) -> Result<ConnectDto, NetCmdError> {
    connect_ticket(&state, &ticket)
}

#[tauri::command]
pub async fn net_respond_incoming(
    state: State<'_, NetState>,
    node_id: String,
    accept: bool,
) -> Result<(), NetCmdError> {
    respond_incoming(&state, &node_id, accept)
}

#[tauri::command]
pub async fn net_pending_incoming(
    state: State<'_, NetState>,
) -> Result<Vec<IncomingRequestDto>, NetCmdError> {
    pending_incoming(&state)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use messenger_core::discovery::testing::{
        next_event, spawn_test_node_with, MemoryKnownPeers, TestIdentity, TestOptions,
    };
    use messenger_core::discovery::{ConnectionState, Direction, FailureReason, KnownPeers};
    use messenger_core::identity::testing::FakeKeychain;
    use messenger_core::identity::{IdentityService, KeychainStore, MemorySettingsStore, Ticket};
    use tokio::sync::broadcast::Receiver;

    use super::*;
    use crate::identity_cmd::{my_info, provision};
    use crate::test_util::assert_forms_absent;

    const WAIT: Duration = Duration::from_secs(10);

    /// A started state plus the fixtures that must outlive it.
    struct Started {
        state: NetState,
        known: Arc<MemoryKnownPeers>,
        id: NodeId,
        identity: TestIdentity,
        keychain: Arc<FakeKeychain>,
        _dir: tempfile::TempDir,
    }

    async fn started_with(name: &str, options: TestOptions) -> Started {
        let known = Arc::new(MemoryKnownPeers::new());
        let rest = spawn_test_node_with(name, options.with_known(known.clone())).await;
        let id = rest.id();
        let state = NetState::default();
        let node = rest.node;
        assert!(state.ensure_started_with(async { Ok(node) }).await.unwrap());
        Started {
            state,
            known,
            id,
            identity: rest.identity,
            keychain: rest.keychain,
            _dir: rest.dir,
        }
    }

    async fn started(name: &str) -> Started {
        started_with(name, TestOptions::default()).await
    }

    fn events(s: &Started) -> Receiver<NetEvent> {
        s.state.node().unwrap().subscribe()
    }

    async fn expect(rx: &mut Receiver<NetEvent>, pred: impl Fn(&NetEvent) -> bool) -> NetEvent {
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

    fn is_state(
        peer: NodeId,
        direction: Direction,
        state: ConnectionState,
    ) -> impl Fn(&NetEvent) -> bool {
        move |ev| {
            matches!(ev, NetEvent::ConnectionState { node_id, direction: d, state: s }
                if *node_id == peer && *d == direction && *s == state)
        }
    }

    fn is_incoming(ev: &NetEvent) -> bool {
        matches!(ev, NetEvent::IncomingRequest(_))
    }

    async fn ticket_of(s: &Started) -> String {
        my_ticket(&s.state).await.unwrap().ticket
    }

    #[test]
    fn errors_serialize_with_type_tag() {
        for (err, tag) in [
            (NetCmdError::NotReady, "NotReady"),
            (NetCmdError::InvalidTicket, "InvalidTicket"),
            (NetCmdError::SelfDial, "SelfDial"),
            (NetCmdError::NoSuchRequest, "NoSuchRequest"),
            (NetCmdError::Internal, "Internal"),
        ] {
            assert_eq!(
                serde_json::to_value(err).unwrap(),
                serde_json::json!({ "type": tag })
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn my_ticket_parses_and_names_this_node() {
        let a = started("Alice").await;
        let dto = my_ticket(&a.state).await.unwrap();
        assert_eq!(Ticket::parse(&dto.ticket).unwrap().node_id, a.id);
        assert!(dto.qr_svg.contains("<svg"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_invalid_ticket_is_invalid_ticket() {
        let a = started("Alice").await;
        for bad in ["", "garbage", "p2pm:", &"x".repeat(10_000)] {
            assert_eq!(
                connect_ticket(&a.state, bad).unwrap_err(),
                NetCmdError::InvalidTicket
            );
        }
        let own = ticket_of(&a).await;
        assert_eq!(
            connect_ticket(&a.state, &own).unwrap_err(),
            NetCmdError::SelfDial
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn respond_unknown_is_no_such_request() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        assert_eq!(
            respond_incoming(&a.state, &b.id.to_string(), true).unwrap_err(),
            NetCmdError::NoSuchRequest
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_node_id_is_no_such_request() {
        let a = started("Alice").await;
        for bad in ["", "not-a-node-id", "\u{202E}", &"f".repeat(4096)] {
            assert_eq!(
                respond_incoming(&a.state, bad, true).unwrap_err(),
                NetCmdError::NoSuchRequest
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ticket_connect_accept_flow_end_to_end() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let (mut ea, mut eb) = (events(&a), events(&b));

        let dialed = connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        assert_eq!(dialed.node_id, a.id.to_string());
        let NetEvent::IncomingRequest(req) = expect(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert_eq!(req.node_id, b.id);
        assert_eq!(req.display_name.as_ref().map(|n| n.as_str()), Some("Bob"));

        respond_incoming(&a.state, &b.id.to_string(), true).unwrap();
        expect(
            &mut ea,
            is_state(b.id, Direction::Inbound, ConnectionState::Connected),
        )
        .await;
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert!(pending_incoming(&a.state).unwrap().is_empty());
        assert!(a.known.is_known(&b.id).unwrap());
        assert!(b.known.is_known(&a.id).unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_incoming_snapshot_lists_open_prompts_and_drops_resolved() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let mut ea = events(&a);
        assert!(pending_incoming(&a.state).unwrap().is_empty());

        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;
        let pending = pending_incoming(&a.state).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].node_id, b.id.to_string());
        assert_eq!(pending[0].display_name.as_deref(), Some("Bob"));

        respond_incoming(&a.state, &b.id.to_string(), false).unwrap();
        assert!(pending_incoming(&a.state).unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dtos_never_contain_key_material() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let mut ea = events(&a);
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        let incoming = expect(&mut ea, is_incoming).await;

        let account = a.identity.load().unwrap().account_id;
        let key = *a.keychain.load_identity_key(&account).unwrap().as_bytes();
        let jsons = [
            serde_json::to_string(&my_ticket(&a.state).await.unwrap()).unwrap(),
            serde_json::to_string(&pending_incoming(&a.state).unwrap()).unwrap(),
            serde_json::to_string(&connect_ticket(&b.state, &ticket_of(&a).await).unwrap())
                .unwrap(),
            event_json(&incoming).1.to_string(),
        ];
        for json in &jsons {
            assert_forms_absent(json, &key);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commands_return_not_ready_before_start() {
        let state = NetState::default();
        assert_eq!(my_ticket(&state).await.unwrap_err(), NetCmdError::NotReady);
        assert_eq!(
            connect_ticket(&state, "x").unwrap_err(),
            NetCmdError::NotReady
        );
        assert_eq!(
            respond_incoming(&state, "x", true).unwrap_err(),
            NetCmdError::NotReady
        );
        assert_eq!(pending_incoming(&state).unwrap_err(), NetCmdError::NotReady);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn net_state_starts_at_most_once() {
        let rest = spawn_test_node_with("Alice", TestOptions::default()).await;
        let node = Mutex::new(Some(rest.node));
        let runs = AtomicUsize::new(0);
        let state = NetState::default();
        let attempt = || async {
            state
                .ensure_started_with(async {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(node.lock().unwrap().take().expect("start future ran twice"))
                })
                .await
                .unwrap()
        };
        let results = futures_join(&attempt).await;
        assert_eq!(results.iter().filter(|started| **started).count(), 1);
        assert!(!attempt().await);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(state.node().is_ok());
    }

    async fn futures_join<F, Fut>(make: &F) -> Vec<bool>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = bool>,
    {
        let (a, b, c, d) = tokio::join!(make(), make(), make(), make());
        vec![a, b, c, d]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_failure_reports_not_ready_and_my_info_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let tickets = LiveTicketProvider::new();
        let settings = Arc::new(MemorySettingsStore::new());
        let identity =
            IdentityService::new(Arc::new(FakeKeychain::new()), settings, tickets, dir.path());
        let provisioned = provision(&identity).unwrap();

        let state = NetState::default();
        let failed = state
            .ensure_started_with(async { Err(StartError::Bind("no network".into())) })
            .await;
        assert!(failed.is_err());
        assert_eq!(my_ticket(&state).await.unwrap_err(), NetCmdError::NotReady);

        let info = my_info(&identity).unwrap();
        assert_eq!(info.ticket, format!("p2pm:{}", provisioned.node_id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dial_adds_known_only_after_hello_reply() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let (mut ea, mut eb) = (events(&a), events(&b));
        let ticket = ticket_of(&a).await;

        connect_ticket(&b.state, &ticket).unwrap();
        expect(&mut ea, is_incoming).await;
        assert!(!b.known.is_known(&a.id).unwrap());

        respond_incoming(&a.state, &b.id.to_string(), false).unwrap();
        expect(
            &mut eb,
            is_state(
                a.id,
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        assert!(b.known.is_empty());
        assert!(a.known.is_empty());

        connect_ticket(&b.state, &ticket).unwrap();
        expect(&mut ea, is_incoming).await;
        assert!(!b.known.is_known(&a.id).unwrap());
        respond_incoming(&a.state, &b.id.to_string(), true).unwrap();
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        assert!(b.known.is_known(&a.id).unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_closes_connections_and_peer_sees_disconnected() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let (mut ea, mut eb) = (events(&a), events(&b));
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;
        respond_incoming(&a.state, &b.id.to_string(), true).unwrap();
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        drop(ea);

        a.state.shutdown_with_timeout(Duration::from_secs(2)).await;
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Disconnected),
        )
        .await;
        assert!(a.state.node().is_err());
        a.state.shutdown_with_timeout(Duration::from_secs(2)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forwarded_event_names_are_the_dd02_names() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let mut ea = events(&a);
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;
        respond_incoming(&a.state, &b.id.to_string(), true).unwrap();

        let seen = Mutex::new(BTreeSet::new());
        let collect = |ev: &NetEvent| {
            forward(ev, |name, _| {
                seen.lock().unwrap().insert(name.to_owned());
            })
        };
        collect(&NetEvent::MyTicketChanged);
        collect(&NetEvent::IncomingRequest(
            messenger_core::discovery::IncomingRequest {
                node_id: b.id,
                display_name: None,
            },
        ));
        expect(&mut ea, |ev| {
            collect(ev);
            is_state(b.id, Direction::Inbound, ConnectionState::Connected)(ev)
        })
        .await;

        let allowed: BTreeSet<String> =
            ["incoming_request", "connection_state", "my_ticket_changed"]
                .map(String::from)
                .into();
        assert_eq!(*seen.lock().unwrap(), allowed);
    }
}
