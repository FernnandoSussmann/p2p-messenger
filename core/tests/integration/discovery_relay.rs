//! Relay fallback with no direct path: both nodes can only reach each other
//! through a local relay, and a recording proxy sits between them and it.
//!
//! The iroh test relay only serves TLS, so this verifies relay TLS plus iroh's
//! QUIC encryption together: the canary Hello name must appear nowhere in the
//! captured bytes. Skipping certificate verification (self-signed relay) is
//! confined to this test.
//!
//! The package name `core` shadows std `core`, which breaks `#[tokio::test]`,
//! so the test runs through an explicit runtime.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use core::discovery::testing::{
    hello_frame, next_event, spawn_test_node_with, TestNode, TestOptions,
};
use core::discovery::{ConnectionState, NetEvent, NodeConfig};
use core::identity::Ticket;
use iroh::tls::CaTlsConfig;
use iroh::RelayUrl;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::Receiver;

const CANARY: &str = "CanaryZ9Q";
const WAIT: Duration = Duration::from_secs(15);

type Capture = Arc<Mutex<Vec<u8>>>;

/// Forwards TCP to `target`, recording every byte in both directions.
async fn recording_proxy(target: std::net::SocketAddr, capture: Capture) -> RelayUrl {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let Ok(server) = TcpStream::connect(target).await else {
                continue;
            };
            let (client_rd, client_wr) = client.into_split();
            let (server_rd, server_wr) = server.into_split();
            tokio::spawn(pipe(client_rd, server_wr, capture.clone()));
            tokio::spawn(pipe(server_rd, client_wr, capture.clone()));
        }
    });
    format!("https://127.0.0.1:{port}").parse().unwrap()
}

async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    capture: Capture,
) {
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                capture.lock().unwrap().extend_from_slice(&buf[..n]);
                if to.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn relay_node(name: &str, relay: &RelayUrl) -> TestNode {
    let mut options = TestOptions::default();
    options.config =
        NodeConfig::relay_only(relay.clone()).with_ca_tls(CaTlsConfig::insecure_skip_verify());
    spawn_test_node_with(name, options).await
}

async fn wait_connected(rx: &mut Receiver<NetEvent>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match next_event(rx, left).await {
            Some(NetEvent::ConnectionState {
                state: ConnectionState::Connected,
                ..
            }) => return,
            Some(_) => {}
            None => panic!("timed out waiting for Connected"),
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn relay_only_connection_carries_no_plaintext() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (_map, relay_url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
            let relay_addr = {
                let host = relay_url.host_str().unwrap();
                let port = relay_url.port_or_known_default().unwrap();
                format!("{host}:{port}").parse().unwrap()
            };
            let capture: Capture = Arc::default();
            let proxy_url = recording_proxy(relay_addr, capture.clone()).await;

            let a = relay_node(CANARY, &proxy_url).await;
            let b = relay_node("Bob", &proxy_url).await;
            let (mut ea, mut eb) = (a.node.subscribe(), b.node.subscribe());

            let ticket_a = a.node.ticket_when_ready(WAIT).await.unwrap();
            b.node.ticket_when_ready(WAIT).await.unwrap();
            let parsed = Ticket::parse(ticket_a.as_str()).unwrap();
            assert!(parsed.addr.relay_urls().next().is_some());
            assert!(parsed.addr.ip_addrs().next().is_none());

            b.node.connect_ticket(ticket_a.as_str()).unwrap();
            loop {
                match next_event(&mut ea, WAIT).await.expect("prompt") {
                    NetEvent::IncomingRequest(_) => break,
                    _ => continue,
                }
            }
            a.node.respond_incoming(&b.id(), true).unwrap();
            wait_connected(&mut ea).await;
            wait_connected(&mut eb).await;

            assert_eq!(a.node.connection_is_relay_only(&b.id()), Some(true));
            assert_eq!(b.node.connection_is_relay_only(&a.id()), Some(true));

            tokio::time::sleep(Duration::from_millis(300)).await;
            let captured = capture.lock().unwrap().clone();
            assert!(!captured.is_empty(), "proxy saw no relay traffic");
            assert!(!contains(&captured, CANARY.as_bytes()));
            assert!(!contains(&captured, &hello_frame(CANARY)));
        });
}
