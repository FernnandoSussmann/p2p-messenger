//! Separate test binary on purpose (DD-06 §3): installs the process-global
//! tracing subscriber; do not merge into `integration`.
//!
//! Runtime canary: with the production log filter at TRACE, two chatting
//! loopback nodes must leave no name, nickname, body, query, key or ticket in
//! the log directory, yet the log must carry connection state with short
//! NodeIds and path addresses. One test per binary, because it installs the
//! global subscriber. The package name `core` shadows std `core`, which breaks
//! `#[tokio::test]`, so the test runs through a runtime builder.

use std::fs;
use std::path::Path;
use std::time::Duration;

use core::identity::KeychainStore;
use core::logging::{self, LogConfig};
use core::messaging::testing::{chat_peer, connect_and_accept, StoreKind};
use core::messaging::MessagingConfig;
use core::storage::Storage;
use tracing::level_filters::LevelFilter;
use uuid::Uuid;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_logs(dir: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        all.extend(fs::read(entry.unwrap().path()).unwrap());
    }
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn logs_carry_connection_facts_and_no_secrets() {
    let logs = tempfile::tempdir().unwrap();
    let mut config = LogConfig::new(logs.path(), LevelFilter::TRACE);
    config.max_file_bytes = 256 * 1024;
    config.max_files = 4;
    tracing::subscriber::set_global_default(logging::subscriber(&config).unwrap()).unwrap();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(logs.path()));
}

async fn run(logs: &Path) {
    let tag = Uuid::new_v4();
    let (name_a, name_b) = (
        format!("canary-name-a-{tag}"),
        format!("canary-name-b-{tag}"),
    );
    let (nick_a, nick_b) = (
        format!("canary-name-na-{tag}"),
        format!("canary-name-nb-{tag}"),
    );
    let body = format!("canary-body-{tag}");
    let query = format!("canary-query-{tag}");

    let config = MessagingConfig::fast;
    let a = chat_peer(&name_a, StoreKind::SqlCipher, config()).await;
    let b = chat_peer(&name_b, StoreKind::SqlCipher, config()).await;
    connect_and_accept(&a, &b).await;
    a.book.get().rename(&b.node.id(), &nick_b).unwrap();
    b.book.get().rename(&a.node.id(), &nick_a).unwrap();

    let sent = a.messaging.send(b.node.id(), body.clone()).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while b
        .store
        .get(&b.node.node.account(), &a.node.id(), &sent.id)
        .unwrap()
        .is_none()
    {
        assert!(tokio::time::Instant::now() < deadline, "body never arrived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for p in [&a, &b] {
        p.store
            .search(&p.node.node.account(), None, &query, 10)
            .unwrap();
    }

    let mut needles: Vec<Vec<u8>> = Vec::new();
    let texts = [&name_a, &name_b, &nick_a, &nick_b, &body, &query];
    for t in texts {
        needles.push(t.as_bytes().to_vec());
        needles.push(hex(t.as_bytes()).into_bytes());
    }
    for p in [&a, &b] {
        let account = p.node.node.account();
        let storage = Storage::new(p.node.dir.path(), p.node.keychain.clone());
        storage.database(&account).unwrap();
        let db = p.node.keychain.load_db_key(&account).unwrap();
        let identity = p.node.keychain.load_identity_key(&account).unwrap();
        for raw in [db.as_bytes(), identity.as_bytes()] {
            needles.push(raw.to_vec());
            needles.push(hex(raw).into_bytes());
            needles.push(hex(raw).to_uppercase().into_bytes());
        }
        let ticket = p.node.node.ticket().unwrap().to_string();
        needles.push(ticket.into_bytes());
        needles.push(p.node.id().to_string().into_bytes());
    }

    let (short_a, short_b) = (
        format!("peer={}", core::logging::fields::peer(&a.node.id())),
        format!("peer={}", core::logging::fields::peer(&b.node.id())),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let text = loop {
        let text = String::from_utf8_lossy(&read_logs(logs)).into_owned();
        let has = |peer: &str, dir: &str| {
            text.lines()
                .any(|l| l.contains("connection state") && l.contains(peer) && l.contains(dir))
        };
        let paths = text.lines().any(|l| {
            l.contains("path opened") && (l.contains("addr=ip:") || l.contains("addr=relay:"))
        });
        if has(&short_b, "direction=Outbound") && has(&short_a, "direction=Inbound") && paths {
            break text;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected connection lines missing:\n{text}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    drop((a, b));
    let all = read_logs(logs);
    assert!(!text.is_empty());
    for (i, needle) in needles.iter().enumerate() {
        assert!(!contains(&all, needle), "needle {i} found in logs");
    }
}
