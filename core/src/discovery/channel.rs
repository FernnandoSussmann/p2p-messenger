//! Authorized channels: the only way the messaging slice reaches a peer.

use std::fmt;
use std::sync::{Arc, Weak};

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};

use super::node::{Inner, CLOSE_PROTOCOL};
use crate::identity::NodeId;

/// An authorized connection to one peer: it exists only after the Hello
/// exchange and the trust gate admitted the peer. Cheap to clone. Holds a
/// `Weak` to the node so it never keeps the node alive.
#[derive(Clone)]
pub struct Channel {
    peer: NodeId,
    conn: Connection,
    stable_id: usize,
    node: Weak<Inner>,
}

/// The channel's connection ended or was replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelClosed;

/// Receives every newly authorized channel. Must be idempotent per
/// (peer, stable_id): a channel may be delivered twice when registration races
/// with authorization. Always called with no node lock held; must not block.
pub trait ChannelHandler: Send + Sync + 'static {
    fn on_channel(&self, channel: Channel);
}

impl Channel {
    pub(super) fn new(peer: NodeId, conn: Connection, node: &Arc<Inner>) -> Self {
        let stable_id = conn.stable_id();
        Self {
            peer,
            conn,
            stable_id,
            node: Arc::downgrade(node),
        }
    }

    pub fn peer(&self) -> NodeId {
        self.peer
    }

    pub fn stable_id(&self) -> usize {
        self.stable_id
    }

    /// True while this is still the node's authorized channel for `peer`:
    /// not replaced, not closed, node not closing.
    pub fn is_current(&self) -> bool {
        self.conn.close_reason().is_none()
            && self
                .node
                .upgrade()
                .is_some_and(|inner| inner.is_current_channel(&self.peer, self.stable_id))
    }

    pub async fn open_stream(&self) -> Result<(SendStream, RecvStream), ChannelClosed> {
        self.conn.open_bi().await.map_err(|_| ChannelClosed)
    }

    pub async fn accept_stream(&self) -> Result<(SendStream, RecvStream), ChannelClosed> {
        self.conn.accept_bi().await.map_err(|_| ChannelClosed)
    }

    pub async fn closed(&self) {
        self.conn.closed().await;
    }

    /// The peer broke the messaging protocol (e.g. oversized frame): close
    /// with `CLOSE_PROTOCOL`.
    pub fn close_protocol(&self) {
        self.conn.close(VarInt::from_u32(CLOSE_PROTOCOL), b"");
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Channel({}#{})", self.peer.fmt_short(), self.stable_id)
    }
}
