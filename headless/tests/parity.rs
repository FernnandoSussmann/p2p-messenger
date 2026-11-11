#![cfg(unix)]
//! Level-3 run of the parity scenarios: the same code as
//! `core/tests/discovery_parity.rs`, against real `messenger-headless`
//! processes driven through the control socket.
//!
//! Scope: host processes run with `--lan off` because loopback multicast is
//! unreliable, so `presence_off_persists_and_stays_dialable` proves presence
//! persistence and dialability here, not LAN visibility. Visibility is covered
//! by `docker/smoke.sh` stage 5 (Level 3) and `discovery_lan.rs` (Level 1).

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use messenger_core::discovery::testing::scenarios::{
    decline_leaves_no_trust, presence_off_persists_and_stays_dialable,
    unknown_prompts_once_then_known_reconnects_silently, NodeDriver,
};
use messenger_headless::control::{self, Request};
use serde_json::Value;
use tempfile::TempDir;
use tokio::process::{Child, Command};

const BIN: &str = env!("CARGO_BIN_EXE_messenger-headless");
const READY: Duration = Duration::from_secs(30);
const STOP: Duration = Duration::from_secs(10);
const LIMIT: Duration = Duration::from_secs(180);

struct HeadlessDriver {
    dir: TempDir,
    name: String,
    child: Option<Child>,
}

impl HeadlessDriver {
    async fn new(name: &str) -> Self {
        let mut driver = Self {
            dir: TempDir::new().expect("temp dir"),
            name: name.to_owned(),
            child: None,
        };
        driver.start().await;
        driver
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    async fn start(&mut self) {
        let child = Command::new(BIN)
            .args(["run", "--data-dir"])
            .arg(self.dir.path())
            .args(["--keyring", "file", "--name", &self.name])
            .args([
                "--relay",
                "disabled",
                "--lan",
                "off",
                "--bind",
                "127.0.0.1:0",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn messenger-headless");
        self.child = Some(child);
        let deadline = tokio::time::Instant::now() + READY;
        while self.request(Request::NodeId).await["ok"] != true {
            assert!(
                tokio::time::Instant::now() < deadline,
                "headless node did not become ready"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn request(&self, request: Request) -> Value {
        control::request(&self.data_dir(), &request).await
    }

    async fn ok(&self, request: Request) -> Result<Value, String> {
        let body = self.request(request).await;
        if body["ok"] == true {
            Ok(body)
        } else {
            Err(body["error"].as_str().unwrap_or("unknown").to_owned())
        }
    }

    async fn string_field(&self, request: Request, field: &str) -> String {
        let body = self.ok(request).await.expect("control request");
        body[field].as_str().expect(field).to_owned()
    }
}

impl NodeDriver for HeadlessDriver {
    async fn node_id(&self) -> String {
        self.string_field(Request::NodeId, "node_id").await
    }

    async fn ticket(&self) -> String {
        self.string_field(Request::Ticket, "ticket").await
    }

    async fn connect(&self, ticket: &str) -> Result<String, String> {
        let body = self
            .ok(Request::Connect {
                ticket: ticket.to_owned(),
            })
            .await?;
        Ok(body["node_id"].as_str().unwrap_or_default().to_owned())
    }

    async fn respond(&self, node_id: &str, accept: bool) -> Result<(), String> {
        self.ok(Request::Respond {
            node_id: node_id.to_owned(),
            accept,
        })
        .await
        .map(drop)
    }

    async fn set_presence(&self, on: bool) -> Result<bool, String> {
        let body = self.ok(Request::Presence { enabled: Some(on) }).await?;
        Ok(body["enabled"] == true)
    }

    async fn presence(&self) -> bool {
        let body = self
            .ok(Request::Presence { enabled: None })
            .await
            .expect("presence");
        body["enabled"] == true
    }

    async fn connected(&self) -> Vec<String> {
        let body = self.ok(Request::Connected).await.expect("connected");
        body["peers"]
            .as_array()
            .expect("peers")
            .iter()
            .map(|p| p.as_str().expect("node id").to_owned())
            .collect()
    }

    async fn wait_event(
        &self,
        name: &str,
        node: Option<&str>,
        state: Option<&str>,
        timeout: Duration,
    ) -> Option<Value> {
        let body = self
            .request(Request::Wait {
                event: name.to_owned(),
                node: node.map(str::to_owned),
                state: state.map(str::to_owned),
                timeout_secs: Some(timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0)),
            })
            .await;
        (body["ok"] == true).then(|| body["payload"].clone())
    }

    async fn restart(&mut self) {
        let _ = self.request(Request::Shutdown).await;
        if let Some(mut child) = self.child.take() {
            if tokio::time::timeout(STOP, child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
        }
        self.start().await;
    }
}

fn run<F: std::future::Future>(fut: F) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(LIMIT, fut)
                .await
                .expect("scenario timed out");
        });
}

async fn pair() -> (HeadlessDriver, HeadlessDriver) {
    (
        HeadlessDriver::new("Alice").await,
        HeadlessDriver::new("Bob").await,
    )
}

#[test]
fn unknown_prompts_once_then_known_reconnects_silently_headless() {
    run(async {
        let (mut a, mut b) = pair().await;
        unknown_prompts_once_then_known_reconnects_silently(&mut a, &mut b).await;
    });
}

#[test]
fn decline_leaves_no_trust_headless() {
    run(async {
        let (mut a, mut b) = pair().await;
        decline_leaves_no_trust(&mut a, &mut b).await;
    });
}

#[test]
fn presence_off_persists_and_stays_dialable_headless() {
    run(async {
        let (mut a, mut b) = pair().await;
        presence_off_persists_and_stays_dialable(&mut a, &mut b).await;
    });
}
