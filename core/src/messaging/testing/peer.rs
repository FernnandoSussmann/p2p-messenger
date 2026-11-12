//! A full chat peer for tests: a discovery node with a contact book, a message
//! store and the messaging engine.

use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use protocol::{MessageId, MAX_ACK_BYTES};
use tokio::time::{timeout, timeout_at, Instant};

use crate::contacts::testing::{BookCell, MemoryContactStore};
use crate::contacts::{accept_incoming, HistoryEraser, NoMessageStore};
use crate::discovery::testing::{next_event, restart, spawn_test_node_with, TestNode, TestOptions};
use crate::discovery::{Channel, ConnectionState, NetEvent, Node};
use crate::identity::NodeId;
use crate::messaging::testing::MemoryMessageStore;
use crate::messaging::{
    decode_ack, open_message_store, MessageStore, Messaging, MessagingConfig, MessagingDeps,
};

const WAIT: Duration = Duration::from_secs(10);
const REPLY_WAIT: Duration = Duration::from_millis(1500);
const CLOSE_PROBE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Memory,
    SqlCipher,
}

pub struct ChatPeer {
    pub node: TestNode,
    pub book: BookCell,
    pub store: Arc<dyn MessageStore>,
    pub messaging: Messaging,
    pub config: MessagingConfig,
    kind: StoreKind,
}

async fn assemble(
    node: TestNode,
    book: BookCell,
    store: Arc<dyn MessageStore>,
    config: MessagingConfig,
    kind: StoreKind,
) -> ChatPeer {
    let deps = MessagingDeps {
        known: node.known.clone(),
        store: store.clone(),
    };
    let messaging = Messaging::start(&node.node, deps, config.clone())
        .await
        .expect("messaging start");
    ChatPeer {
        node,
        book,
        store,
        messaging,
        config,
        kind,
    }
}

async fn sqlcipher_store(node: &TestNode) -> Arc<dyn MessageStore> {
    Arc::new(open_message_store(&node.identity).expect("open store").1)
}

/// A provisioned node whose contacts live in memory. `Memory` shares one
/// in-memory message store with the book (as its history eraser); `SqlCipher`
/// opens the encrypted store once the identity is provisioned.
pub async fn chat_peer(name: &str, kind: StoreKind, config: MessagingConfig) -> ChatPeer {
    let book = BookCell::new();
    let contacts = Arc::new(MemoryContactStore::new());
    let (memory, eraser): (Option<Arc<dyn MessageStore>>, Arc<dyn HistoryEraser>) = match kind {
        StoreKind::Memory => {
            let store: Arc<dyn MessageStore> = Arc::new(MemoryMessageStore::new());
            (Some(store.clone()), store)
        }
        StoreKind::SqlCipher => (None, Arc::new(NoMessageStore)),
    };
    let options = TestOptions::default().with_known_factory(book.known_factory(contacts, eraser));
    let node = spawn_test_node_with(name, options).await;
    let store = match memory {
        Some(store) => store,
        None => sqlcipher_store(&node).await,
    };
    assemble(node, book, store, config, kind).await
}

/// Stops the peer and starts it again over the same store and directory.
pub async fn restart_chat_peer(old: ChatPeer) -> ChatPeer {
    let ChatPeer {
        node,
        book,
        store,
        messaging,
        config,
        kind,
    } = old;
    messaging.shutdown();
    let node = restart(node).await;
    let store = match kind {
        StoreKind::Memory => store,
        StoreKind::SqlCipher => {
            drop(store);
            sqlcipher_store(&node).await
        }
    };
    assemble(node, book, store, config, kind).await
}

/// `a` dials `b` by ticket. If `b` has no contact for `a` yet it accepts the
/// prompt. Returns once both sides are `Connected`.
pub async fn connect_and_accept(a: &ChatPeer, b: &ChatPeer) {
    let (mut ea, mut eb) = (a.node.node.subscribe(), b.node.node.subscribe());
    let ticket = b.node.node.ticket().expect("ticket").to_string();
    a.node.node.connect_ticket(&ticket).expect("dial");
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match next_event(&mut eb, left).await {
            Some(NetEvent::IncomingRequest(_)) => {
                accept_incoming(&b.book.get(), &b.node.node, &a.node.id(), "peer").expect("accept");
            }
            Some(e) if connected(&e, &a.node.id()) => break,
            Some(_) => {}
            None => panic!("peer b never connected"),
        }
    }
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match next_event(&mut ea, left).await {
            Some(e) if connected(&e, &b.node.id()) => break,
            Some(_) => {}
            None => panic!("peer a never connected"),
        }
    }
    while !(a.messaging.has_channel(&b.node.id()) && b.messaging.has_channel(&a.node.id())) {
        assert!(Instant::now() < deadline, "engines never saw the channel");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn connected(event: &NetEvent, peer: &NodeId) -> bool {
    matches!(event, NetEvent::ConnectionState { node_id, state: ConnectionState::Connected, .. }
        if node_id == peer)
}

/// What the receiving side did with a raw frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawReply {
    pub ack: Option<MessageId>,
    pub reset: bool,
    pub connection_closed: bool,
}

/// Writes `bytes` on an already opened stream and reads the reply.
pub async fn send_on_stream(
    channel: &Channel,
    mut send: SendStream,
    mut recv: RecvStream,
    bytes: &[u8],
) -> RawReply {
    let _ = send.write_all(bytes).await;
    let _ = send.finish();
    read_reply(channel, &mut recv).await
}

/// Opens a stream on `node`'s authorized channel to `peer` and writes raw
/// bytes: the only way tests inject adversarial or hand-timestamped frames.
pub async fn send_raw_frame(node: &Node, peer: &NodeId, bytes: &[u8]) -> RawReply {
    let channel = node.channel(peer).expect("an authorized channel");
    match channel.open_stream().await {
        Ok((send, recv)) => send_on_stream(&channel, send, recv, bytes).await,
        Err(_) => RawReply {
            ack: None,
            reset: false,
            connection_closed: true,
        },
    }
}

async fn read_reply(channel: &Channel, recv: &mut RecvStream) -> RawReply {
    let deadline = Instant::now() + REPLY_WAIT;
    match timeout_at(deadline, recv.read_to_end(MAX_ACK_BYTES)).await {
        Ok(Ok(bytes)) => RawReply {
            ack: decode_ack(&bytes).ok(),
            reset: false,
            connection_closed: false,
        },
        Ok(Err(_)) => {
            let closed = timeout(CLOSE_PROBE, channel.closed()).await.is_ok();
            RawReply {
                ack: None,
                reset: !closed,
                connection_closed: closed,
            }
        }
        Err(_) => RawReply {
            ack: None,
            reset: false,
            connection_closed: false,
        },
    }
}
