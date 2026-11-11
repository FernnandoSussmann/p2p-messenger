//! The control surface: a Unix domain socket, one JSON request line in, one
//! JSON response line out. Test harness only.

use std::future::Future;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

use crate::cli::CtlCommand;

pub const SOCKET_NAME: &str = "control.sock";
pub const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_LIMIT: u64 = 1024 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const DEFAULT_WAIT_SECS: u64 = 30;
const CLIENT_SLACK: Duration = Duration::from_secs(10);

const KNOWN_COMMANDS: [&str; 9] = [
    "node-id",
    "ticket",
    "presence",
    "peers",
    "connect",
    "respond",
    "connected",
    "wait",
    "shutdown",
];

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    NodeId,
    Ticket,
    Presence {
        #[serde(default)]
        enabled: Option<bool>,
    },
    Peers,
    Connect {
        ticket: String,
    },
    Respond {
        node_id: String,
        accept: bool,
        #[serde(default)]
        nickname: Option<String>,
    },
    Connected,
    Wait {
        event: String,
        #[serde(default)]
        node: Option<String>,
        #[serde(default)]
        state: Option<String>,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    Shutdown,
}

/// A response line plus whether the process should stop after sending it.
pub struct Reply {
    pub body: Value,
    pub stop: bool,
}

impl Reply {
    pub fn ok(fields: Value) -> Self {
        let mut body = json!({ "ok": true });
        if let (Some(base), Value::Object(extra)) = (body.as_object_mut(), fields) {
            base.extend(extra);
        }
        Self { body, stop: false }
    }

    pub fn err(name: &str) -> Self {
        Self {
            body: json!({ "ok": false, "error": name }),
            stop: false,
        }
    }

    pub fn stopping(mut self) -> Self {
        self.stop = true;
        self
    }
}

impl From<&CtlCommand> for Request {
    fn from(cmd: &CtlCommand) -> Self {
        match cmd {
            CtlCommand::NodeId => Request::NodeId,
            CtlCommand::Ticket => Request::Ticket,
            CtlCommand::Presence(enabled) => Request::Presence { enabled: *enabled },
            CtlCommand::Peers => Request::Peers,
            CtlCommand::Connect(ticket) => Request::Connect {
                ticket: ticket.clone(),
            },
            CtlCommand::Respond {
                node_id,
                accept,
                nickname,
            } => Request::Respond {
                node_id: node_id.clone(),
                accept: *accept,
                nickname: nickname.clone(),
            },
            CtlCommand::Connected => Request::Connected,
            CtlCommand::Wait(w) => Request::Wait {
                event: w.event.name().to_owned(),
                node: w.node.clone(),
                state: w.state.clone(),
                timeout_secs: w.timeout_secs,
            },
            CtlCommand::Shutdown => Request::Shutdown,
        }
    }
}

pub fn wait_timeout(timeout_secs: Option<u64>) -> Duration {
    Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_WAIT_SECS))
}

pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SOCKET_NAME)
}

fn parse_request(line: &[u8]) -> Result<Request, &'static str> {
    let value: Value = serde_json::from_slice(line).map_err(|_| "BadRequest")?;
    let cmd = value
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or("BadRequest")?
        .to_owned();
    serde_json::from_value(value).map_err(|_| {
        if KNOWN_COMMANDS.contains(&cmd.as_str()) {
            "BadRequest"
        } else {
            "UnknownCommand"
        }
    })
}

#[derive(Debug)]
pub struct BindError(pub String);

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BindError {}

/// Refuses a symlink or non-socket at `path`, removes a stale socket, and
/// binds a fresh one with mode 0600.
pub async fn bind(path: &Path) -> Result<UnixListener, BindError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if UnixStream::connect(path).await.is_ok() {
                return Err(BindError(
                    "another node is already serving this data dir".into(),
                ));
            }
            std::fs::remove_file(path)
                .map_err(|e| BindError(format!("cannot remove stale control socket: {e}")))?;
        }
        Ok(_) => {
            return Err(BindError(
                "control socket path exists and is not a socket (symlink or file)".into(),
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(BindError(format!(
                "cannot inspect control socket path: {e}"
            )))
        }
    }
    let listener = UnixListener::bind(path)
        .map_err(|e| BindError(format!("cannot bind control socket: {e}")))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| BindError(format!("cannot restrict control socket: {e}")))?;
    Ok(listener)
}

/// Accepts connections until the task is aborted. A reply with `stop` set
/// notifies `stop` after its line has been written.
pub async fn serve<F, Fut>(listener: UnixListener, handler: F, stop: Arc<Notify>)
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Reply> + Send + 'static,
{
    let handler = Arc::new(handler);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let handler = Arc::clone(&handler);
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            let _ = handle_connection(stream, handler, stop).await;
        });
    }
}

async fn read_line(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES as u64 + 1));
    let mut line = Vec::new();
    let read = tokio::time::timeout(READ_TIMEOUT, reader.read_until(b'\n', &mut line)).await;
    match read {
        Ok(Ok(_)) if line.len() <= MAX_REQUEST_BYTES => Some(line),
        _ => None,
    }
}

async fn handle_connection<F, Fut>(
    mut stream: UnixStream,
    handler: Arc<F>,
    stop: Arc<Notify>,
) -> std::io::Result<()>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Reply> + Send + 'static,
{
    let reply = match read_line(&mut stream).await {
        None => Reply::err("BadRequest"),
        Some(line) => match parse_request(&line) {
            Ok(request) => handler(request).await,
            Err(name) => Reply::err(name),
        },
    };
    let mut out = serde_json::to_vec(&reply.body).unwrap_or_default();
    out.push(b'\n');
    let written = stream.write_all(&out).await;
    let _ = stream.shutdown().await;
    if reply.stop {
        stop.notify_one();
    }
    written?;
    // Discard what an oversized or garbage request left unread, so closing
    // does not reset the connection before the client reads the response.
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
        let mut sink = tokio::io::sink();
        let _ = tokio::io::copy(&mut (&mut stream).take(DRAIN_LIMIT), &mut sink).await;
    })
    .await;
    Ok(())
}

/// Sends one request to the node serving `data_dir`, prints the response and
/// returns the process exit code (0 if `ok`, 1 otherwise).
pub async fn ctl(data_dir: &Path, command: &CtlCommand) -> i32 {
    let body = request(data_dir, &Request::from(command)).await;
    println!("{body}");
    i32::from(body.get("ok") != Some(&Value::Bool(true)))
}

/// Sends one request and returns the response body; transport failures become
/// `{"ok":false,"error":…}`.
pub async fn request(data_dir: &Path, request: &Request) -> Value {
    let timeout = match request {
        Request::Wait { timeout_secs, .. } => wait_timeout(*timeout_secs) + CLIENT_SLACK,
        _ => CLIENT_SLACK,
    };
    request_with_timeout(data_dir, request, timeout).await
}

async fn request_with_timeout(data_dir: &Path, request: &Request, timeout: Duration) -> Value {
    match tokio::time::timeout(timeout, exchange(data_dir, request)).await {
        Ok(Ok(body)) => body,
        Ok(Err(name)) => json!({ "ok": false, "error": name }),
        Err(_) => json!({ "ok": false, "error": "Timeout" }),
    }
}

async fn exchange(data_dir: &Path, request: &Request) -> Result<Value, &'static str> {
    let mut stream = UnixStream::connect(socket_path(data_dir))
        .await
        .map_err(|_| "NotRunning")?;
    let mut line = serde_json::to_vec(request).map_err(|_| "BadRequest")?;
    line.push(b'\n');
    stream.write_all(&line).await.map_err(|_| "Io")?;
    stream.shutdown().await.map_err(|_| "Io")?;
    let mut raw = Vec::new();
    (&mut stream)
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut raw)
        .await
        .map_err(|_| "Io")?;
    serde_json::from_slice(&raw).map_err(|_| "BadResponse")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{WaitArgs, WaitEvent};

    #[test]
    fn requests_round_trip_from_ctl_commands() {
        let cases = [
            CtlCommand::NodeId,
            CtlCommand::Presence(Some(false)),
            CtlCommand::Connect("p2pm:x".into()),
            CtlCommand::Respond {
                node_id: "n".into(),
                accept: true,
                nickname: Some("Bobby".into()),
            },
            CtlCommand::Wait(WaitArgs {
                event: WaitEvent::ConnectionState,
                node: Some("n".into()),
                state: Some("connected".into()),
                timeout_secs: Some(3),
            }),
            CtlCommand::Shutdown,
        ];
        for cmd in &cases {
            let request = Request::from(cmd);
            let line = serde_json::to_vec(&request).unwrap();
            assert_eq!(parse_request(&line).unwrap(), request);
        }
    }

    #[test]
    fn parse_distinguishes_bad_and_unknown() {
        assert_eq!(parse_request(b"not json"), Err("BadRequest"));
        assert_eq!(parse_request(b"[1]"), Err("BadRequest"));
        assert_eq!(parse_request(b"{}"), Err("BadRequest"));
        assert_eq!(parse_request(br#"{"cmd":"nope"}"#), Err("UnknownCommand"));
        assert_eq!(parse_request(br#"{"cmd":"connect"}"#), Err("BadRequest"));
        assert_eq!(
            parse_request(br#"{"cmd":"presence"}"#),
            Ok(Request::Presence { enabled: None })
        );
    }
}
