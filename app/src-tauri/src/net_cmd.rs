//! The network Tauri commands, the managed `NetState` and the event forwarder.
//!
//! The logic lives in plain functions over `NetState` so it can be tested
//! without a Tauri runtime; the `#[tauri::command]` wrappers only forward the
//! managed state. Event names and payloads come from core's `event_json`.

use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use iroh::RelayUrl;
use messenger_core::contacts::{accept_incoming, AcceptError, ContactError};
use messenger_core::discovery::{
    event_json, ConnectError, IncomingRequestDto, KnownPeers, LanPeerDto, LiveTicketProvider,
    MyTicketDto, NetEvent, Node, NodeConfig, NodeDeps, PresenceDto, PresenceError, RelayChoice,
    RespondError, StartError,
};
use messenger_core::identity::{
    ticket_to_qr_svg, validate_relay_url, AccountId, FileSettingsStore, IdentityService,
    IdentityStatus, KeychainStore, NodeId, SettingsError, SettingsStore, TicketError,
    TicketProvider,
};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::RecvError;

use crate::contacts_cmd::ContactsState;
use crate::identity_cmd::{AppIdentity, DisplayNameKind};

const TICKET_READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Webview-facing error. Messages never embed backend or secret details.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum NetCmdError {
    NotReady,
    InvalidTicket,
    SelfDial,
    NoSuchRequest,
    SettingsUnavailable,
    InvalidNickname { kind: DisplayNameKind },
    ContactsFull,
    ContactsUnavailable,
    Internal,
}

impl From<AcceptError> for NetCmdError {
    fn from(err: AcceptError) -> Self {
        match err {
            AcceptError::InvalidNickname(e)
            | AcceptError::Contacts(ContactError::InvalidNickname(e)) => {
                Self::InvalidNickname { kind: e.into() }
            }
            AcceptError::NoSuchRequest | AcceptError::Contacts(ContactError::NotFound) => {
                Self::NoSuchRequest
            }
            AcceptError::Contacts(ContactError::Full) => Self::ContactsFull,
            AcceptError::Contacts(
                ContactError::SelfContact
                | ContactError::AlreadyExists
                | ContactError::Unavailable
                | ContactError::HistoryEraseFailed,
            ) => Self::ContactsUnavailable,
        }
    }
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

impl From<PresenceError> for NetCmdError {
    fn from(_: PresenceError) -> Self {
        Self::SettingsUnavailable
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

/// What `MESSENGER_RELAY_URL` asks for at launch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RelayOverride {
    /// Unset: leave the stored relay as it is.
    #[default]
    Keep,
    /// The literal `default`: forget the stored relay, use the default mesh.
    Clear,
    /// A valid relay URL (canonical form), stored for this account.
    Set(String),
}

/// `MESSENGER_RELAY_URL` was set to something that is neither `default` nor a
/// valid relay URL. Startup must fail rather than ignore it.
#[derive(Debug, PartialEq, Eq)]
pub struct RelayEnvError;

impl std::fmt::Display for RelayEnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "MESSENGER_RELAY_URL must be `default` or an https relay URL \
             (http only for a loopback host)",
        )
    }
}

impl std::error::Error for RelayEnvError {}

pub fn relay_override_from_env(raw: Option<String>) -> Result<RelayOverride, RelayEnvError> {
    match raw {
        None => Ok(RelayOverride::Keep),
        Some(v) if v == "default" => Ok(RelayOverride::Clear),
        Some(v) => validate_relay_url(&v)
            .map(RelayOverride::Set)
            .map_err(|_| RelayEnvError),
    }
}

fn apply_relay_override(
    settings: &dyn SettingsStore,
    account: &AccountId,
    requested: &RelayOverride,
) -> Result<(), SettingsError> {
    match requested {
        RelayOverride::Keep => Ok(()),
        RelayOverride::Clear => settings.set_relay_url(account, None),
        RelayOverride::Set(url) => settings.set_relay_url(account, Some(url)),
    }
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
    relay_override: RelayOverride,
}

impl Default for NetState {
    fn default() -> Self {
        Self::new(RelayOverride::Keep)
    }
}

impl NetState {
    pub fn new(relay_override: RelayOverride) -> Self {
        Self {
            node: RwLock::new(None),
            phase: tokio::sync::Mutex::new(Phase::Idle),
            relay_override,
        }
    }

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

/// Decline answers the prompt only. Accept is "add contact": it persists an
/// Unverified contact under the user's nickname, then answers the prompt.
pub async fn respond_incoming(
    state: &NetState,
    contacts: &ContactsState,
    node_id: &str,
    accept: bool,
    nickname: Option<String>,
) -> Result<(), NetCmdError> {
    let node = state.node()?;
    let peer = NodeId::from_str(node_id).map_err(|_| NetCmdError::NoSuchRequest)?;
    if !accept {
        node.respond_incoming(&peer, false)?;
        return Ok(());
    }
    let nickname = nickname.ok_or(NetCmdError::InvalidNickname {
        kind: DisplayNameKind::Empty,
    })?;
    let book = contacts.book().ok_or(NetCmdError::ContactsUnavailable)?;
    tauri::async_runtime::spawn_blocking(move || {
        accept_incoming(&book, node.as_ref(), &peer, &nickname)
    })
    .await
    .map_err(|_| NetCmdError::Internal)?
    .map(|_| ())
    .map_err(NetCmdError::from)
}

pub fn pending_incoming(state: &NetState) -> Result<Vec<IncomingRequestDto>, NetCmdError> {
    Ok(state
        .node()?
        .pending_incoming()
        .iter()
        .map(IncomingRequestDto::from)
        .collect())
}

pub fn presence_get(state: &NetState) -> Result<PresenceDto, NetCmdError> {
    Ok(PresenceDto {
        enabled: state.node()?.presence(),
    })
}

pub fn presence_set(state: &NetState, enabled: bool) -> Result<PresenceDto, NetCmdError> {
    let node = state.node()?;
    node.set_presence(enabled)?;
    Ok(PresenceDto {
        enabled: node.presence(),
    })
}

pub fn lan_peers(state: &NetState) -> Result<Vec<LanPeerDto>, NetCmdError> {
    Ok(state
        .node()?
        .lan_peers()
        .iter()
        .map(LanPeerDto::from)
        .collect())
}

/// Saves the name through the identity service, then re-publishes it on the
/// LAN if the node runs.
pub fn set_display_name_and_refresh<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
    state: &NetState,
    name: &str,
) -> Result<(), crate::identity_cmd::IdentityCmdError> {
    crate::identity_cmd::set_display_name(service, name)?;
    if let Ok(node) = state.node() {
        node.refresh_local_name();
    }
    Ok(())
}

/// Maps a core event to its external name and payload and hands it to `emit`.
pub fn forward(ev: &NetEvent, emit: impl Fn(&str, Value)) {
    let (name, payload) = event_json(ev);
    emit(name, payload);
}

/// Everything `ensure_started` needs besides the identity service.
pub struct NetBoot {
    pub settings: Arc<FileSettingsStore>,
    pub tickets: LiveTicketProvider,
}

/// Starts the node once if an identity exists. Contacts are bound as soon as
/// the identity is read, before and independently of the start, so a relay or
/// bind failure still leaves them usable. The relay override is applied
/// here, after the account exists and before `Node::start`, so an override
/// never lands without an `account_id` to key it. A stored relay that cannot
/// be read or saved refuses the start rather than falling back to the default
/// mesh. Returns whether this call started a node.
pub async fn start_if_present<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    state: &NetState,
    identity: &IdentityService<K, S, T>,
    settings: Arc<dyn SettingsStore + Send + Sync>,
    tickets: LiveTicketProvider,
    contacts: &ContactsState,
    mut config: NodeConfig,
) -> bool {
    let (account, known) = match identity.status() {
        IdentityStatus::Present {
            account_id,
            node_id,
        } => (
            account_id,
            contacts.bind(account_id, node_id) as Arc<dyn KnownPeers>,
        ),
        _ => return false,
    };
    let start = async {
        let relay_err = |what: &str| StartError::Bind(format!("relay setting {what}"));
        apply_relay_override(&*settings, &account, &state.relay_override)
            .map_err(|_| relay_err("could not be saved"))?;
        let stored = settings
            .get_relay_url(&account)
            .map_err(|_| relay_err("is unreadable"))?;
        if let Some(url) = stored {
            let url = RelayUrl::from_str(&url).map_err(|_| relay_err("is invalid"))?;
            config.relay = RelayChoice::Custom(url);
        }
        let deps = NodeDeps {
            settings: Arc::clone(&settings),
            known,
            tickets,
        };
        Node::start(identity, deps, config).await
    };
    matches!(state.ensure_started_with(start).await, Ok(true))
}

/// Starts the node in the background, once, if an identity exists. A start
/// failure leaves the app usable: commands report `NotReady`.
pub fn ensure_started(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NetState>();
        let boot = app.state::<NetBoot>();
        let identity = app.state::<AppIdentity>();
        let contacts = app.state::<Arc<ContactsState>>();
        let started = start_if_present(
            &state,
            &*identity,
            boot.settings.clone(),
            boot.tickets.clone(),
            &contacts,
            NodeConfig::desktop(),
        )
        .await;
        if !started {
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
    contacts: State<'_, Arc<ContactsState>>,
    node_id: String,
    accept: bool,
    nickname: Option<String>,
) -> Result<(), NetCmdError> {
    respond_incoming(&state, &contacts, &node_id, accept, nickname).await
}

#[tauri::command]
pub async fn net_pending_incoming(
    state: State<'_, NetState>,
) -> Result<Vec<IncomingRequestDto>, NetCmdError> {
    pending_incoming(&state)
}

#[tauri::command]
pub async fn net_presence_get(state: State<'_, NetState>) -> Result<PresenceDto, NetCmdError> {
    presence_get(&state)
}

#[tauri::command]
pub async fn net_presence_set(
    state: State<'_, NetState>,
    enabled: bool,
) -> Result<PresenceDto, NetCmdError> {
    presence_set(&state, enabled)
}

#[tauri::command]
pub async fn net_lan_peers(state: State<'_, NetState>) -> Result<Vec<LanPeerDto>, NetCmdError> {
    lan_peers(&state)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use messenger_core::contacts::testing::{MemoryContactStore, RecordingHistoryEraser};
    use messenger_core::contacts::TrustState;
    use messenger_core::discovery::testing::{
        spawn_test_node_with, spawn_test_node_wrapped, MemoryLanBus, SettingsWrapper, TestIdentity,
        TestOptions,
    };
    use messenger_core::discovery::{ConnectionState, Direction, FailureReason};
    use messenger_core::identity::testing::FakeKeychain;
    use messenger_core::identity::{
        DisplayName, IdentityService, KeychainStore, MemorySettingsStore, Ticket,
    };

    use super::*;
    use crate::identity_cmd::{my_info, provision};
    use crate::test_util::{
        assert_forms_absent, events, expect, is_incoming, is_state, started, started_with,
        ticket_of, Started, WAIT,
    };

    async fn accept(s: &Started, peer: &NodeId, nickname: &str) -> Result<(), NetCmdError> {
        respond_incoming(
            &s.state,
            &s.contacts,
            &peer.to_string(),
            true,
            Some(nickname.to_owned()),
        )
        .await
    }

    async fn decline(s: &Started, peer: &NodeId) -> Result<(), NetCmdError> {
        respond_incoming(&s.state, &s.contacts, &peer.to_string(), false, None).await
    }

    fn book(s: &Started) -> Arc<messenger_core::contacts::ContactBook> {
        s.contacts.book().expect("contacts are bound")
    }

    #[test]
    fn errors_serialize_with_type_tag() {
        for (err, tag) in [
            (NetCmdError::NotReady, "NotReady"),
            (NetCmdError::InvalidTicket, "InvalidTicket"),
            (NetCmdError::SelfDial, "SelfDial"),
            (NetCmdError::NoSuchRequest, "NoSuchRequest"),
            (NetCmdError::SettingsUnavailable, "SettingsUnavailable"),
            (NetCmdError::ContactsFull, "ContactsFull"),
            (NetCmdError::ContactsUnavailable, "ContactsUnavailable"),
            (NetCmdError::Internal, "Internal"),
        ] {
            assert_eq!(
                serde_json::to_value(err).unwrap(),
                serde_json::json!({ "type": tag })
            );
        }
    }

    #[test]
    fn invalid_nickname_error_carries_its_kind() {
        let err = NetCmdError::InvalidNickname {
            kind: DisplayNameKind::InvisibleCharacter,
        };
        assert_eq!(
            serde_json::to_value(err).unwrap(),
            serde_json::json!({ "type": "InvalidNickname", "kind": "InvisibleCharacter" })
        );
    }

    #[test]
    fn accept_maps_every_contact_error() {
        use messenger_core::identity::DisplayNameError as D;
        let kind = |k| NetCmdError::InvalidNickname { kind: k };
        let cases = [
            (
                AcceptError::InvalidNickname(D::Empty),
                kind(DisplayNameKind::Empty),
            ),
            (
                AcceptError::Contacts(ContactError::InvalidNickname(D::BidiOverride)),
                kind(DisplayNameKind::BidiOverride),
            ),
            (AcceptError::NoSuchRequest, NetCmdError::NoSuchRequest),
            (
                AcceptError::Contacts(ContactError::NotFound),
                NetCmdError::NoSuchRequest,
            ),
            (
                AcceptError::Contacts(ContactError::Full),
                NetCmdError::ContactsFull,
            ),
            (
                AcceptError::Contacts(ContactError::SelfContact),
                NetCmdError::ContactsUnavailable,
            ),
            (
                AcceptError::Contacts(ContactError::AlreadyExists),
                NetCmdError::ContactsUnavailable,
            ),
            (
                AcceptError::Contacts(ContactError::Unavailable),
                NetCmdError::ContactsUnavailable,
            ),
            (
                AcceptError::Contacts(ContactError::HistoryEraseFailed),
                NetCmdError::ContactsUnavailable,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(NetCmdError::from(err), want);
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
            accept(&a, &b.id, "Bobby").await.unwrap_err(),
            NetCmdError::NoSuchRequest
        );
        assert_eq!(
            decline(&a, &b.id).await.unwrap_err(),
            NetCmdError::NoSuchRequest
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_node_id_is_no_such_request() {
        let a = started("Alice").await;
        for bad in ["", "not-a-node-id", "\u{202E}", &"f".repeat(4096)] {
            for (accept, nickname) in [(true, Some("Bobby".to_owned())), (false, Some("x".into()))]
            {
                assert_eq!(
                    respond_incoming(&a.state, &a.contacts, bad, accept, nickname)
                        .await
                        .unwrap_err(),
                    NetCmdError::NoSuchRequest
                );
            }
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

        accept(&a, &b.id, "Bobby").await.unwrap();
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
        let theirs = book(&a).get(&b.id).unwrap().unwrap();
        assert_eq!(theirs.nickname.as_str(), "Bobby");
        assert_eq!(theirs.self_name.as_ref().map(|n| n.as_str()), Some("Bob"));
        assert_eq!(theirs.trust, TrustState::Unverified);
        let ours = book(&b).get(&a.id).unwrap().unwrap();
        assert_eq!(ours.nickname.as_str(), "Alice");
        assert_eq!(ours.trust, TrustState::Unverified);
        assert!(a.history.calls().is_empty() && b.history.calls().is_empty());
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
        assert_eq!(pending[0].suggested_nickname, "Bob");

        decline(&a, &b.id).await.unwrap();
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
        let contacts = started("Carol").await.contacts;
        assert_eq!(
            respond_incoming(&state, &contacts, "x", true, None)
                .await
                .unwrap_err(),
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
    async fn dial_remembers_peer_only_after_valid_reply() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let (mut ea, mut eb) = (events(&a), events(&b));
        let ticket = ticket_of(&a).await;

        connect_ticket(&b.state, &ticket).unwrap();
        expect(&mut ea, is_incoming).await;
        assert!(book(&b).list().unwrap().is_empty());

        decline(&a, &b.id).await.unwrap();
        expect(
            &mut eb,
            is_state(
                a.id,
                Direction::Outbound,
                ConnectionState::Failed(FailureReason::Declined),
            ),
        )
        .await;
        assert!(book(&b).list().unwrap().is_empty());
        assert!(book(&a).list().unwrap().is_empty());

        connect_ticket(&b.state, &ticket).unwrap();
        expect(&mut ea, is_incoming).await;
        assert!(!book(&b).is_known(&a.id).unwrap());
        accept(&a, &b.id, "Bobby").await.unwrap();
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
        let remembered = book(&b).get(&a.id).unwrap().unwrap();
        assert_eq!(remembered.nickname.as_str(), "Alice");
        assert_eq!(remembered.trust, TrustState::Unverified);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accept_requires_a_valid_nickname() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let mut ea = events(&a);
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;

        let long = "a".repeat(129);
        let cases = [
            (None, DisplayNameKind::Empty),
            (Some(""), DisplayNameKind::Empty),
            (Some("a\nb"), DisplayNameKind::ControlCharacter),
            (Some("\u{202E}x"), DisplayNameKind::BidiOverride),
            (Some("a\u{200B}b"), DisplayNameKind::InvisibleCharacter),
            (Some(long.as_str()), DisplayNameKind::TooLong),
        ];
        for (nickname, kind) in cases {
            let err = respond_incoming(
                &a.state,
                &a.contacts,
                &b.id.to_string(),
                true,
                nickname.map(str::to_owned),
            )
            .await
            .unwrap_err();
            assert_eq!(err, NetCmdError::InvalidNickname { kind }, "{nickname:?}");
        }
        assert_eq!(pending_incoming(&a.state).unwrap().len(), 1);
        assert!(book(&a).list().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn contacts_bound_even_if_node_start_fails() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, settings, tickets) = provisioned_identity(&dir);
        identity.provision().unwrap();
        let broken = Arc::new(FlakyPresence {
            inner: settings,
            fail: false,
            fail_relay: true,
        });
        let contacts = test_contacts();
        let state = NetState::default();
        assert!(contacts.book().is_none());

        let started = start_if_present(
            &state,
            &identity,
            broken,
            tickets,
            &contacts,
            NodeConfig::loopback(),
        )
        .await;
        assert!(!started);
        assert!(state.node().is_err());
        assert!(contacts.book().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_closes_connections_and_peer_sees_disconnected() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let (mut ea, mut eb) = (events(&a), events(&b));
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;
        accept(&a, &b.id, "Bobby").await.unwrap();
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

    /// Delegates to a memory store and can be told to fail `set_presence`
    /// and `get_relay_url`.
    struct FlakyPresence {
        inner: Arc<MemorySettingsStore>,
        fail: bool,
        fail_relay: bool,
    }

    impl SettingsStore for FlakyPresence {
        fn set_display_name(&self, id: &AccountId, n: &DisplayName) -> Result<(), SettingsError> {
            self.inner.set_display_name(id, n)
        }
        fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
            self.inner.get_display_name(id)
        }
        fn set_provisioned_at(&self, id: &AccountId, ms: i64) -> Result<(), SettingsError> {
            self.inner.set_provisioned_at(id, ms)
        }
        fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
            self.inner.get_provisioned_at(id)
        }
        fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
            self.inner.get_presence(id)
        }
        fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
            if self.fail {
                return Err(SettingsError::Unavailable);
            }
            self.inner.set_presence(id, on)
        }
        fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
            if self.fail_relay {
                return Err(SettingsError::Unavailable);
            }
            self.inner.get_relay_url(id)
        }
        fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
            self.inner.set_relay_url(id, url)
        }
    }

    async fn lan_started(name: &str, bus: &MemoryLanBus) -> Started {
        started_with(name, TestOptions::default().with_lan(bus)).await
    }

    async fn until_lan(s: &Started, pred: impl Fn(&[LanPeerDto]) -> bool) -> Vec<LanPeerDto> {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let current = lan_peers(&s.state).unwrap();
            if pred(&current) {
                return current;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the LAN list: {current:?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn lists(peers: &[LanPeerDto], id: NodeId) -> bool {
        peers.iter().any(|p| p.node_id == id.to_string())
    }

    type Fixture = (TestIdentity, Arc<MemorySettingsStore>, LiveTicketProvider);

    fn provisioned_identity(dir: &tempfile::TempDir) -> Fixture {
        let settings = Arc::new(MemorySettingsStore::new());
        let tickets = LiveTicketProvider::new();
        let identity = IdentityService::new(
            Arc::new(FakeKeychain::new()),
            settings.clone(),
            tickets.clone(),
            dir.path(),
        );
        (identity, settings, tickets)
    }

    fn test_contacts() -> ContactsState {
        ContactsState::new(
            Arc::new(MemoryContactStore::new()),
            Arc::new(RecordingHistoryEraser::new()),
        )
    }

    async fn start_with_override(
        state: &NetState,
        identity: &TestIdentity,
        settings: &Arc<MemorySettingsStore>,
        tickets: &LiveTicketProvider,
    ) -> bool {
        start_if_present(
            state,
            identity,
            settings.clone(),
            tickets.clone(),
            &test_contacts(),
            NodeConfig::loopback(),
        )
        .await
    }

    fn account_of(identity: &TestIdentity) -> AccountId {
        match identity.status() {
            IdentityStatus::Present { account_id, .. } => account_id,
            _ => panic!("no identity"),
        }
    }

    #[test]
    fn relay_env_unset_keeps() {
        assert_eq!(relay_override_from_env(None), Ok(RelayOverride::Keep));
    }

    #[test]
    fn relay_env_valid_parses_to_canonical_set() {
        assert_eq!(
            relay_override_from_env(Some("https://relay.example".into())),
            Ok(RelayOverride::Set("https://relay.example/".into()))
        );
    }

    #[test]
    fn relay_env_default_is_clear() {
        assert_eq!(
            relay_override_from_env(Some("default".into())),
            Ok(RelayOverride::Clear)
        );
    }

    #[test]
    fn relay_env_invalid_is_startup_error() {
        for bad in [
            "",
            "Default",
            "garbage",
            "http://relay.example",
            "ftp://relay.example",
        ] {
            assert_eq!(
                relay_override_from_env(Some(bad.into())),
                Err(RelayEnvError),
                "{bad:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_env_valid_is_set_and_stored_on_start() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, settings, tickets) = provisioned_identity(&dir);
        identity.provision().unwrap();
        let account = account_of(&identity);
        let state = NetState::new(RelayOverride::Set("https://relay.example/".into()));

        assert!(start_with_override(&state, &identity, &settings, &tickets).await);
        assert_eq!(
            settings.get_relay_url(&account).unwrap().as_deref(),
            Some("https://relay.example/")
        );
        state.shutdown_with_timeout(Duration::from_secs(2)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_env_default_clears_stored_value() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, settings, tickets) = provisioned_identity(&dir);
        identity.provision().unwrap();
        let account = account_of(&identity);
        settings
            .set_relay_url(&account, Some("https://old.example"))
            .unwrap();
        let state = NetState::new(RelayOverride::Clear);

        assert!(start_with_override(&state, &identity, &settings, &tickets).await);
        assert_eq!(settings.get_relay_url(&account).unwrap(), None);
        state.shutdown_with_timeout(Duration::from_secs(2)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_env_unset_leaves_stored_value() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, settings, tickets) = provisioned_identity(&dir);
        identity.provision().unwrap();
        let account = account_of(&identity);
        settings
            .set_relay_url(&account, Some("https://kept.example"))
            .unwrap();
        let state = NetState::default();

        assert!(start_with_override(&state, &identity, &settings, &tickets).await);
        assert_eq!(
            settings.get_relay_url(&account).unwrap().as_deref(),
            Some("https://kept.example/")
        );
        state.shutdown_with_timeout(Duration::from_secs(2)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_override_applied_only_once_account_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, settings, tickets) = provisioned_identity(&dir);
        let state = NetState::new(RelayOverride::Set("https://relay.example/".into()));
        let contacts = test_contacts();
        let started = start_if_present(
            &state,
            &identity,
            settings.clone(),
            tickets.clone(),
            &contacts,
            NodeConfig::loopback(),
        )
        .await;
        assert!(!started);
        assert!(state.node().is_err());
        assert!(contacts.book().is_none(), "no book without an identity");

        let provisioned = identity.provision().unwrap();
        assert!(start_with_override(&state, &identity, &settings, &tickets).await);
        assert_eq!(
            settings
                .get_relay_url(&provisioned.account_id)
                .unwrap()
                .as_deref(),
            Some("https://relay.example/")
        );
        state.shutdown_with_timeout(Duration::from_secs(2)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rename_refreshes_lan_name() {
        let bus = MemoryLanBus::new();
        let a = lan_started("Alice", &bus).await;
        let b = lan_started("Bob", &bus).await;
        until_lan(&b, |p| lists(p, a.id)).await;

        set_display_name_and_refresh(&a.identity, &a.state, "Alicia").unwrap();
        until_lan(&b, |p| {
            p.iter().any(|d| {
                d.node_id == a.id.to_string() && d.display_name.as_deref() == Some("Alicia")
            })
        })
        .await;

        a.state.node().unwrap().set_presence(false).unwrap();
        until_lan(&b, |p| !lists(p, a.id)).await;
        set_display_name_and_refresh(&a.identity, &a.state, "Alina").unwrap();
        assert_eq!(bus.advertised_record(&a.id), None);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!lists(&lan_peers(&b.state).unwrap(), a.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rename_without_a_node_still_saves() {
        let dir = tempfile::tempdir().unwrap();
        let (identity, _settings, _tickets) = provisioned_identity(&dir);
        identity.provision().unwrap();
        let state = NetState::default();
        set_display_name_and_refresh(&identity, &state, "Ada").unwrap();
        assert_eq!(
            my_info(&identity).unwrap().display_name.as_deref(),
            Some("Ada")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn presence_get_defaults_on_and_set_persists() {
        let bus = MemoryLanBus::new();
        let spy: Arc<Mutex<Option<Arc<MemorySettingsStore>>>> = Arc::default();
        let wrapper: SettingsWrapper = {
            let spy = spy.clone();
            Arc::new(move |inner| {
                *spy.lock().unwrap() = Some(inner.clone());
                inner
            })
        };
        let rest = spawn_test_node_wrapped(
            "Alice",
            TestOptions::default().with_lan(&bus),
            Some(wrapper),
        )
        .await;
        let account = rest.identity.load().unwrap().account_id;
        let state = NetState::default();
        let node = rest.node;
        state.ensure_started_with(async { Ok(node) }).await.unwrap();

        assert!(presence_get(&state).unwrap().enabled);
        assert!(!presence_set(&state, false).unwrap().enabled);
        assert!(!presence_get(&state).unwrap().enabled);
        let store = spy.lock().unwrap().clone().unwrap();
        assert_eq!(store.get_presence(&account).unwrap(), Some(false));
        assert!(presence_set(&state, true).unwrap().enabled);
        assert_eq!(store.get_presence(&account).unwrap(), Some(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn presence_set_failure_maps_to_settings_unavailable() {
        let bus = MemoryLanBus::new();
        let wrapper: SettingsWrapper = Arc::new(|inner| {
            Arc::new(FlakyPresence {
                inner,
                fail: true,
                fail_relay: false,
            })
        });
        let rest = spawn_test_node_wrapped(
            "Alice",
            TestOptions::default().with_lan(&bus),
            Some(wrapper),
        )
        .await;
        let state = NetState::default();
        let node = rest.node;
        state.ensure_started_with(async { Ok(node) }).await.unwrap();

        assert_eq!(
            presence_set(&state, false).unwrap_err(),
            NetCmdError::SettingsUnavailable
        );
        assert!(presence_get(&state).unwrap().enabled);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn presence_and_lan_commands_report_not_ready_before_start() {
        let state = NetState::default();
        assert_eq!(presence_get(&state).unwrap_err(), NetCmdError::NotReady);
        assert_eq!(
            presence_set(&state, true).unwrap_err(),
            NetCmdError::NotReady
        );
        assert_eq!(lan_peers(&state).unwrap_err(), NetCmdError::NotReady);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lan_peers_dto_is_unverified_and_has_parseable_ticket() {
        let bus = MemoryLanBus::new();
        let a = lan_started("Alice", &bus).await;
        let b = lan_started("Bob", &bus).await;
        let peers = until_lan(&b, |p| lists(p, a.id)).await;

        let dto = peers
            .iter()
            .find(|p| p.node_id == a.id.to_string())
            .unwrap();
        assert!(!dto.verified);
        assert_eq!(dto.display_name.as_deref(), Some("Alice"));
        assert_eq!(Ticket::parse(&dto.ticket).unwrap().node_id, a.id);
        assert!(!lists(&peers, b.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lan_and_ticket_paths_share_one_trust_gate_end_to_end() {
        let bus = MemoryLanBus::new();
        let a = lan_started("Alice", &bus).await;
        let b = lan_started("Bob", &bus).await;
        let c = lan_started("Carol", &bus).await;
        let (mut ea, mut eb, mut ec) = (events(&a), events(&b), events(&c));

        let picked = until_lan(&b, |p| lists(p, a.id)).await;
        let ticket = &picked
            .iter()
            .find(|p| p.node_id == a.id.to_string())
            .unwrap()
            .ticket;
        connect_ticket(&b.state, ticket).unwrap();
        let NetEvent::IncomingRequest(req) = expect(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert_eq!(req.node_id, b.id);
        assert!(!book(&b).is_known(&a.id).unwrap());
        accept(&a, &b.id, "Bobby").await.unwrap();
        expect(
            &mut eb,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;

        assert!(!presence_set(&a.state, false).unwrap().enabled);
        until_lan(&b, |p| !lists(p, a.id)).await;
        until_lan(&a, |p| lists(p, b.id)).await;

        connect_ticket(&c.state, &ticket_of(&a).await).unwrap();
        let NetEvent::IncomingRequest(req) = expect(&mut ea, is_incoming).await else {
            unreachable!()
        };
        assert_eq!(req.node_id, c.id);
        accept(&a, &c.id, "Carol").await.unwrap();
        expect(
            &mut ec,
            is_state(a.id, Direction::Outbound, ConnectionState::Connected),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forwarded_event_names_are_the_dd02_names() {
        let a = started("Alice").await;
        let b = started("Bob").await;
        let mut ea = events(&a);
        connect_ticket(&b.state, &ticket_of(&a).await).unwrap();
        expect(&mut ea, is_incoming).await;
        accept(&a, &b.id, "Bobby").await.unwrap();

        let seen = Mutex::new(BTreeSet::new());
        let collect = |ev: &NetEvent| {
            forward(ev, |name, _| {
                seen.lock().unwrap().insert(name.to_owned());
            })
        };
        collect(&NetEvent::MyTicketChanged);
        collect(&NetEvent::LanPeersChanged(Vec::new()));
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

        let allowed: BTreeSet<String> = [
            "incoming_request",
            "connection_state",
            "my_ticket_changed",
            "lan_peers_changed",
        ]
        .map(String::from)
        .into();
        assert_eq!(*seen.lock().unwrap(), allowed);
    }
}
