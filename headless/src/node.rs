//! Wires the shipping `core` discovery stack to the control surface.
//!
//! There is deliberately no option that skips the trust prompt, auto-accepts
//! or weakens encryption: `respond` is the only way to accept a peer.

use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::RelayUrl;
use messenger_core::discovery::{
    event_json, ConnectError, FileKnownPeers, LanMode, LanPeerDto, LiveTicketProvider, Node,
    NodeConfig, NodeDeps, RelayChoice,
};
use messenger_core::identity::{FileSettingsStore, IdentityService, IdentityStatus, NodeId};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Notify;

use crate::cli::{LanArg, RelayArg, RunArgs};
use crate::control::{self, Reply, Request};
use crate::file_keychain::FileKeychain;

const EVENT_BUFFER: usize = 256;
const TICKET_READY_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const WAIT_EVENTS: [&str; 3] = ["incoming_request", "connection_state", "lan_peers_changed"];

type HeadlessIdentity = IdentityService<FileKeychain, Arc<FileSettingsStore>, LiveTicketProvider>;

struct Logged {
    name: &'static str,
    payload: Value,
}

struct LogInner {
    buf: VecDeque<Logged>,
}

/// The last [`EVENT_BUFFER`] unconsumed events, with a wake-up for waiters.
pub struct EventLog {
    inner: Mutex<LogInner>,
    notify: Notify,
}

pub struct EventFilter<'a> {
    pub name: &'a str,
    pub node: Option<&'a str>,
    pub state: Option<&'a str>,
}

impl EventFilter<'_> {
    fn matches(&self, entry: &Logged) -> bool {
        if entry.name != self.name {
            return false;
        }
        let node_ok = self.node.is_none_or(|want| match &entry.payload {
            Value::Array(peers) => peers.iter().any(|p| p["node_id"] == want),
            payload => payload["node_id"] == want,
        });
        let state_ok = self.state.is_none_or(|want| entry.payload["state"] == want);
        node_ok && state_ok
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LogInner {
                buf: VecDeque::new(),
            }),
            notify: Notify::new(),
        }
    }

    pub fn push(&self, name: &'static str, payload: Value) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.buf.push_back(Logged { name, payload });
            if inner.buf.len() > EVENT_BUFFER {
                inner.buf.pop_front();
            }
        }
        self.notify.notify_waiters();
    }

    /// Removes and returns the oldest buffered event that matches, else the
    /// first live one, as `{"event":…,"payload":…}`; `None` on timeout. Each
    /// event is returned at most once, like `NodeDriver::wait_event`.
    pub async fn wait(&self, filter: &EventFilter<'_>, timeout: Duration) -> Option<Value> {
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut inner = self.inner.lock().ok()?;
                if let Some(at) = inner.buf.iter().position(|e| filter.matches(e)) {
                    let e = inner.buf.remove(at)?;
                    return Some(json!({ "event": e.name, "payload": e.payload }));
                }
            }
            tokio::select! {
                () = &mut notified => {}
                () = &mut deadline => return None,
            }
        }
    }
}

#[derive(Debug)]
pub struct RunError(pub String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RunError {}

fn fail<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> RunError + '_ {
    move |e| RunError(format!("{what}: {e}"))
}

fn node_config(args: &RunArgs) -> Result<NodeConfig, RunError> {
    let mut config = NodeConfig::desktop().with_internet_lookup(false);
    config.relay = match &args.relay {
        RelayArg::Default => RelayChoice::Default,
        RelayArg::Disabled => RelayChoice::Disabled,
        RelayArg::Custom(url) => {
            RelayChoice::Custom(RelayUrl::from_str(url).map_err(fail("invalid --relay"))?)
        }
    };
    config.lan = match args.lan {
        LanArg::Mdns => LanMode::Mdns,
        LanArg::Off => LanMode::Off,
    };
    config.bind_addr = args.bind;
    Ok(config)
}

fn prepare_data_dir(dir: &std::path::Path) -> Result<(), RunError> {
    std::fs::create_dir_all(dir).map_err(fail("cannot create data dir"))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(fail("cannot restrict data dir"))
}

/// Starts the node and serves the control socket until `shutdown` or a signal.
pub async fn run(args: RunArgs) -> Result<(), RunError> {
    let config = node_config(&args)?;
    prepare_data_dir(&args.data_dir)?;
    let listener = control::bind(&control::socket_path(&args.data_dir))
        .await
        .map_err(|e| RunError(e.to_string()))?;
    let keychain =
        FileKeychain::open(args.data_dir.join("keyring")).map_err(fail("cannot open keyring"))?;
    eprintln!(
        "TEST-ONLY headless node: throwaway identity, not for real use; \
         secrets stored unencrypted in {}",
        keychain.dir().display()
    );

    let settings = Arc::new(FileSettingsStore::new(args.data_dir.clone()));
    let tickets = LiveTicketProvider::new();
    let identity: HeadlessIdentity = IdentityService::new(
        keychain,
        Arc::clone(&settings),
        tickets.clone(),
        args.data_dir.clone(),
    );
    if matches!(identity.status(), IdentityStatus::Absent) {
        identity.provision().map_err(fail("cannot provision"))?;
        identity
            .set_display_name(args.name.as_str())
            .map_err(fail("cannot set display name"))?;
    }
    let IdentityStatus::Present { account_id, .. } = identity.status() else {
        return Err(RunError("identity is unavailable".into()));
    };
    let deps = NodeDeps {
        settings,
        known: Arc::new(FileKnownPeers::new(args.data_dir.clone(), account_id)),
        tickets,
    };
    let node = Arc::new(
        Node::start(&identity, deps, config)
            .await
            .map_err(fail("cannot start node"))?,
    );
    eprintln!("node id: {}", node.node_id());

    let log = Arc::new(EventLog::new());
    let mut events = node.subscribe();
    let pump_log = Arc::clone(&log);
    let pump = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    let (name, payload) = event_json(&ev);
                    let line = json!({ "event": name, "payload": payload });
                    let _ = writeln!(std::io::stdout().lock(), "{line}");
                    pump_log.push(name, payload);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let stop = Arc::new(Notify::new());
    let handler_node = Arc::clone(&node);
    let handler_log = Arc::clone(&log);
    let server = tokio::spawn(control::serve(
        listener,
        move |request| handle(Arc::clone(&handler_node), Arc::clone(&handler_log), request),
        Arc::clone(&stop),
    ));

    wait_for_stop(&stop).await;
    server.abort();
    pump.abort();
    let _ = std::fs::remove_file(control::socket_path(&args.data_dir));
    let _ = server.await;
    if let Ok(node) = Arc::try_unwrap(node) {
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, node.shutdown()).await;
    }
    Ok(())
}

async fn wait_for_stop(stop: &Notify) {
    use tokio::signal::unix::{signal, SignalKind};
    let term = signal(SignalKind::terminate());
    let term_fut = async {
        match term {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        () = stop.notified() => {}
        _ = tokio::signal::ctrl_c() => {}
        () = term_fut => {}
    }
}

async fn handle(node: Arc<Node>, log: Arc<EventLog>, request: Request) -> Reply {
    match request {
        Request::NodeId => Reply::ok(json!({ "node_id": node.node_id().to_string() })),
        Request::Ticket => match node.ticket_when_ready(TICKET_READY_TIMEOUT).await {
            Ok(ticket) => Reply::ok(json!({ "ticket": ticket.to_string() })),
            Err(_) => Reply::err("Internal"),
        },
        Request::Presence { enabled } => {
            if let Some(on) = enabled {
                if node.set_presence(on).is_err() {
                    return Reply::err("SettingsUnavailable");
                }
            }
            Reply::ok(json!({ "enabled": node.presence() }))
        }
        Request::Peers => {
            let peers: Vec<LanPeerDto> = node.lan_peers().iter().map(LanPeerDto::from).collect();
            Reply::ok(json!({ "peers": peers }))
        }
        Request::Connect { ticket } => match node.connect_ticket(&ticket) {
            Ok(id) => Reply::ok(json!({ "node_id": id.to_string() })),
            Err(ConnectError::InvalidTicket) => Reply::err("InvalidTicket"),
            Err(ConnectError::SelfDial) => Reply::err("SelfDial"),
            Err(ConnectError::ShuttingDown) => Reply::err("NotReady"),
        },
        Request::Respond { node_id, accept } => {
            let Ok(peer) = NodeId::from_str(&node_id) else {
                return Reply::err("NoSuchRequest");
            };
            match node.respond_incoming(&peer, accept) {
                Ok(()) => Reply::ok(json!({})),
                Err(_) => Reply::err("NoSuchRequest"),
            }
        }
        Request::Connected => {
            let peers: Vec<String> = node
                .connected_peers()
                .iter()
                .map(ToString::to_string)
                .collect();
            Reply::ok(json!({ "peers": peers }))
        }
        Request::Wait {
            event,
            node: want_node,
            state,
            timeout_secs,
        } => {
            if !WAIT_EVENTS.contains(&event.as_str()) {
                return Reply::err("BadRequest");
            }
            let filter = EventFilter {
                name: &event,
                node: want_node.as_deref(),
                state: state.as_deref(),
            };
            match log.wait(&filter, control::wait_timeout(timeout_secs)).await {
                Some(found) => Reply::ok(found),
                None => Reply::err("Timeout"),
            }
        }
        Request::Shutdown => Reply::ok(json!({})).stopping(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_disables_internet_lookup_but_desktop_keeps_it() {
        let args = RunArgs {
            data_dir: "/tmp/x".into(),
            name: messenger_core::identity::validate_display_name("T").unwrap(),
            relay: RelayArg::Default,
            lan: LanArg::Off,
            bind: None,
        };
        assert!(!node_config(&args).unwrap().internet_lookup());
        assert!(NodeConfig::desktop().internet_lookup());
    }

    fn filter<'a>(name: &'a str, node: Option<&'a str>, state: Option<&'a str>) -> EventFilter<'a> {
        EventFilter { name, node, state }
    }

    #[tokio::test]
    async fn wait_returns_buffered_then_live_events() {
        let log = Arc::new(EventLog::new());
        log.push(
            "connection_state",
            json!({"node_id":"a","state":"connecting"}),
        );
        let got = log
            .wait(
                &filter("connection_state", None, None),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(got["payload"]["state"], "connecting");
        assert!(
            log.wait(
                &filter("connection_state", None, None),
                Duration::from_millis(50)
            )
            .await
            .is_none(),
            "a returned event is consumed"
        );

        let waiter = {
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                log.wait(
                    &filter("connection_state", Some("a"), Some("connected")),
                    Duration::from_secs(5),
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        log.push(
            "connection_state",
            json!({"node_id":"b","state":"connected"}),
        );
        log.push(
            "connection_state",
            json!({"node_id":"a","state":"connected"}),
        );
        let got = waiter.await.unwrap().unwrap();
        assert_eq!(got["payload"]["node_id"], "a");
    }

    #[tokio::test]
    async fn wait_times_out_and_buffer_is_capped() {
        let log = EventLog::new();
        assert!(log
            .wait(
                &filter("incoming_request", None, None),
                Duration::from_millis(50)
            )
            .await
            .is_none());
        for i in 0..EVENT_BUFFER + 10 {
            log.push("lan_peers_changed", json!([{ "node_id": i.to_string() }]));
        }
        assert_eq!(log.inner.lock().unwrap().buf.len(), EVENT_BUFFER);
        assert!(log
            .wait(
                &filter("lan_peers_changed", Some("0"), None),
                Duration::from_millis(50)
            )
            .await
            .is_none());
        assert!(log
            .wait(
                &filter("lan_peers_changed", Some("265"), None),
                Duration::from_millis(50)
            )
            .await
            .is_some());
    }
}
