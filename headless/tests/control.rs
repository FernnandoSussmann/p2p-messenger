#![cfg(unix)]

use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use messenger_core::discovery::{
    event_json, ConnectionState, Direction, IncomingRequest, NetEvent,
};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_messenger-headless");

struct Headless {
    child: Child,
    dir: PathBuf,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn capture(mut source: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = source.read(&mut chunk) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });
    buf
}

fn run_args(dir: &Path, name: &str) -> Vec<String> {
    [
        "run",
        "--data-dir",
        dir.to_str().unwrap(),
        "--keyring",
        "file",
        "--name",
        name,
        "--relay",
        "disabled",
        "--lan",
        "off",
        "--bind",
        "127.0.0.1:0",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn ctl(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(BIN)
        .args(["ctl", "--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    (out.status.code().unwrap(), body)
}

impl Headless {
    fn start(dir: &Path, name: &str) -> Self {
        let mut child = Command::new(BIN)
            .args(run_args(dir, name))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = capture(child.stdout.take().unwrap());
        let stderr = capture(child.stderr.take().unwrap());
        let node = Self {
            child,
            dir: dir.to_owned(),
            stdout,
            stderr,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if ctl(dir, &["node-id"]).0 == 0 {
                return node;
            }
            assert!(Instant::now() < deadline, "node did not come up");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn ctl(&self, args: &[&str]) -> Value {
        ctl(&self.dir, args).1
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("control.sock")
    }

    fn stop(mut self) {
        assert_eq!(ctl(&self.dir, &["shutdown"]).0, 0);
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "node did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn raw_exchange(sock: &Path, payload: &[u8]) -> Value {
    let mut stream = UnixStream::connect(sock).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = stream.write_all(payload);
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    serde_json::from_slice(&raw).unwrap_or(Value::Null)
}

#[test]
fn control_socket_is_unix_only_and_private() {
    let tmp = tempfile::tempdir().unwrap();
    let node = Headless::start(&tmp.path().join("data"), "Alice");

    let mode = std::fs::metadata(node.socket())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    #[cfg(target_os = "linux")]
    {
        let pid = node.child.id();
        let mine: HashSet<String> = std::fs::read_dir(format!("/proc/{pid}/fd"))
            .unwrap()
            .filter_map(|e| std::fs::read_link(e.unwrap().path()).ok())
            .filter_map(|l| {
                let l = l.to_string_lossy().into_owned();
                l.strip_prefix("socket:[")
                    .and_then(|s| s.strip_suffix(']'))
                    .map(str::to_owned)
            })
            .collect();
        assert!(!mine.is_empty());
        for table in ["tcp", "tcp6"] {
            let text = std::fs::read_to_string(format!("/proc/{pid}/net/{table}")).unwrap();
            for line in text.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                let (state, inode) = (cols[3], cols[9]);
                assert!(
                    !(state == "0A" && mine.contains(inode)),
                    "process owns a TCP listener: {line}"
                );
            }
        }
    }

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(src).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!text.contains("TcpListener"));
    }
    node.stop();
}

#[test]
fn oversized_or_garbage_control_request_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let node = Headless::start(&tmp.path().join("data"), "Alice");
    let sock = node.socket();

    let bad = json!({ "ok": false, "error": "BadRequest" });
    let mut oversized = vec![b'a'; 64 * 1024];
    oversized.push(b'\n');
    assert_eq!(raw_exchange(&sock, &oversized), bad);
    assert_eq!(raw_exchange(&sock, b"}{ not json\n"), bad);
    assert_eq!(raw_exchange(&sock, b"[1,2]\n"), bad);
    assert_eq!(raw_exchange(&sock, b"{\"cmd\":\"connect\"}\n"), bad);
    assert_eq!(
        raw_exchange(&sock, b"{\"cmd\":\"frobnicate\"}\n"),
        json!({ "ok": false, "error": "UnknownCommand" })
    );

    let (code, body) = ctl(&node.dir, &["node-id"]);
    assert_eq!(code, 0);
    assert_eq!(body["ok"], true);
    node.stop();
}

fn refused(args: &[&str]) -> std::process::Output {
    Command::new(BIN).args(args).output().unwrap()
}

#[test]
fn run_refuses_without_keyring_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data");
    let out = refused(&["run", "--data-dir", dir.to_str().unwrap(), "--name", "A"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!dir.exists());
}

#[test]
fn run_refuses_os_keyring() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data");
    let out = refused(&[
        "run",
        "--data-dir",
        dir.to_str().unwrap(),
        "--keyring",
        "os",
        "--name",
        "A",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!dir.exists());
}

#[test]
fn run_refuses_symlinked_socket_path() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data");
    std::fs::create_dir(&dir).unwrap();
    let target = tmp.path().join("target");
    std::fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, dir.join("control.sock")).unwrap();

    let out = Command::new(BIN)
        .args(run_args(&dir, "A"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    assert!(std::fs::symlink_metadata(dir.join("control.sock"))
        .unwrap()
        .file_type()
        .is_symlink());

    std::fs::remove_file(dir.join("control.sock")).unwrap();
    std::fs::write(dir.join("control.sock"), b"file").unwrap();
    let out = Command::new(BIN)
        .args(run_args(&dir, "A"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn banner_says_test_only() {
    let tmp = tempfile::tempdir().unwrap();
    let node = Headless::start(&tmp.path().join("data"), "Alice");
    let stderr = String::from_utf8_lossy(&node.stderr.lock().unwrap()).into_owned();
    assert!(stderr.contains("TEST-ONLY"), "stderr: {stderr}");
    node.stop();
}

fn start_pair(tmp: &Path) -> (Headless, Headless) {
    (
        Headless::start(&tmp.join("a"), "Alice"),
        Headless::start(&tmp.join("b"), "Bob"),
    )
}

fn connect_and_accept(a: &Headless, b: &Headless) {
    connect_and_accept_as(a, b, &[]);
}

fn connect_and_accept_as(a: &Headless, b: &Headless, nickname: &[&str]) {
    let ticket = a.ctl(&["ticket"])["ticket"].as_str().unwrap().to_owned();
    let a_id = a.ctl(&["node-id"])["node_id"].as_str().unwrap().to_owned();
    let b_id = b.ctl(&["node-id"])["node_id"].as_str().unwrap().to_owned();
    assert_eq!(b.ctl(&["connect", &ticket])["node_id"], a_id.as_str());

    let req = a.ctl(&["wait", "incoming_request", "--timeout-secs", "30"]);
    assert_eq!(req["payload"]["node_id"], b_id.as_str());
    let mut respond = vec!["respond", b_id.as_str(), "accept"];
    respond.extend(nickname);
    assert_eq!(a.ctl(&respond)["ok"], true);
    for (node, peer) in [(a, &b_id), (b, &a_id)] {
        let up = node.ctl(&[
            "wait",
            "connection_state",
            "--node",
            peer,
            "--state",
            "connected",
            "--timeout-secs",
            "30",
        ]);
        assert_eq!(up["ok"], true, "{up}");
    }
    assert_eq!(a.ctl(&["connected"])["peers"], json!([b_id]));
}

fn nicknames(node: &Headless) -> Vec<String> {
    let reply = node.ctl(&["contacts"]);
    reply["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["nickname"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn respond_accept_with_and_without_nickname() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    connect_and_accept_as(&a, &b, &["Bobby"]);
    assert_eq!(nicknames(&a), ["Bobby"]);
    assert_eq!(nicknames(&b), ["Alice"]);
    a.stop();
    b.stop();

    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    connect_and_accept(&a, &b);
    assert_eq!(nicknames(&a), ["Bob"]);

    let ticket = a.ctl(&["ticket"])["ticket"].as_str().unwrap().to_owned();
    let c = Headless::start(&tmp.path().join("c"), "Carol");
    let c_id = c.ctl(&["node-id"])["node_id"].as_str().unwrap().to_owned();
    c.ctl(&["connect", &ticket]);
    a.ctl(&[
        "wait",
        "incoming_request",
        "--node",
        &c_id,
        "--timeout-secs",
        "30",
    ]);
    let bad = a.ctl(&["respond", &c_id, "accept", "a\u{200B}b"]);
    assert_eq!(bad["error"], "InvalidNickname");
    assert_eq!(nicknames(&a), ["Bob"]);
    assert_eq!(a.ctl(&["respond", &c_id, "decline"])["ok"], true);
    a.stop();
    b.stop();
    c.stop();
}

#[test]
fn contacts_query_lists_nicknames() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    assert_eq!(a.ctl(&["contacts"]), json!({"ok": true, "contacts": []}));
    connect_and_accept_as(&a, &b, &["Bobby"]);
    let b_id = b.ctl(&["node-id"])["node_id"].as_str().unwrap().to_owned();
    assert_eq!(
        a.ctl(&["contacts"]),
        json!({"ok": true, "contacts": [{"node_id": b_id, "nickname": "Bobby"}]})
    );
    a.stop();
    b.stop();
}

#[test]
fn headless_persists_contacts_encrypted() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    connect_and_accept_as(&a, &b, &["Bobby"]);
    a.stop();
    b.stop();

    let dir_a = tmp.path().join("a");
    let again = Headless::start(&dir_a, "Alice");
    assert_eq!(nicknames(&again), ["Bobby"]);
    again.stop();
    assert!(dir_a.join("messenger.db").is_file());
    assert!(!dir_a.join("contacts.json").exists());
    assert!(!dir_a.join("settings.json").exists());
    let db = std::fs::read(dir_a.join("messenger.db")).unwrap();
    assert!(!String::from_utf8_lossy(&db).contains("Bobby"));
}

#[test]
fn stdout_event_names_are_the_dd02_names() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    connect_and_accept(&a, &b);

    let id = iroh::SecretKey::from_bytes(&[1; 32]).public();
    let known: HashSet<&str> = [
        NetEvent::IncomingRequest(IncomingRequest {
            node_id: id,
            display_name: None,
        }),
        NetEvent::ConnectionState {
            node_id: id,
            direction: Direction::Inbound,
            state: ConnectionState::Connected,
            display_name: None,
        },
        NetEvent::MyTicketChanged,
        NetEvent::LanPeersChanged(Vec::new()),
    ]
    .iter()
    .map(|ev| event_json(ev).0)
    .collect();

    let mut seen = HashSet::new();
    for node in [&a, &b] {
        let out = String::from_utf8(node.stdout.lock().unwrap().clone()).unwrap();
        for line in out.lines() {
            let v: Value = serde_json::from_str(line).expect("stdout line is JSON");
            let name = v["event"].as_str().expect("event name").to_owned();
            assert!(known.contains(name.as_str()), "unexpected event {name}");
            seen.insert(name);
        }
    }
    assert!(seen.contains("incoming_request"));
    assert!(seen.contains("connection_state"));
}

#[test]
fn stdout_events_contain_no_key_material() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = start_pair(tmp.path());
    connect_and_accept(&a, &b);

    let mut canaries: Vec<Vec<u8>> = Vec::new();
    for node in [&a, &b] {
        for entry in std::fs::read_dir(node.dir.join("keyring")).unwrap() {
            let key = std::fs::read(entry.unwrap().path()).unwrap();
            assert_eq!(key.len(), 32);
            let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
            canaries.push(hex.into_bytes());
            canaries.push(key);
        }
    }
    assert_eq!(canaries.len(), 8);

    let mut haystacks: Vec<Vec<u8>> = Vec::new();
    for node in [&a, &b] {
        haystacks.push(node.stdout.lock().unwrap().clone());
        haystacks.push(node.stderr.lock().unwrap().clone());
    }
    let dirs: Vec<PathBuf> = [&a, &b].iter().map(|n| n.dir.clone()).collect();
    a.stop();
    b.stop();
    for dir in dirs {
        let mut stack = vec![dir];
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap() {
                let p = entry.unwrap().path();
                let meta = std::fs::symlink_metadata(&p).unwrap();
                if meta.is_dir() {
                    if p.file_name().is_none_or(|n| n != "keyring") {
                        stack.push(p);
                    }
                } else if meta.is_file() {
                    haystacks.push(std::fs::read(&p).unwrap());
                }
            }
        }
    }
    for hay in &haystacks {
        for canary in &canaries {
            assert!(
                !hay.windows(canary.len()).any(|w| w == canary.as_slice()),
                "key material found in output or data dir"
            );
        }
    }
}
