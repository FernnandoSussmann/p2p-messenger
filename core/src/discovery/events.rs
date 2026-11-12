//! Events the discovery slice emits to its subscribers.

use super::lan::LanPeer;
use crate::identity::{DisplayName, NodeId};

/// Something observable happened on the network layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetEvent {
    /// An unknown peer is asking to connect and awaits Accept/Decline.
    IncomingRequest(IncomingRequest),
    /// A connection to `node_id` changed state.
    ConnectionState {
        node_id: NodeId,
        direction: Direction,
        state: ConnectionState,
        /// Hello name of the peer: `Some` only on `Connected` with a valid
        /// name. Advisory.
        display_name: Option<DisplayName>,
    },
    /// The live ticket changed; consumers re-fetch it.
    MyTicketChanged,
    /// The set of peers visible on the LAN changed; carries the full list.
    LanPeersChanged(Vec<LanPeer>),
}

/// A pending request from an unknown peer. The name is self-asserted and
/// UNVERIFIED: show it as a hint, never trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingRequest {
    pub node_id: NodeId,
    pub display_name: Option<DisplayName>,
}

/// Who opened the connection. Inbound = the peer dialed us (trust gate side);
/// Outbound = we dialed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Failed(FailureReason),
}

/// Why a connection attempt or an established connection failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    Declined,
    Timeout,
    Unreachable,
    Protocol,
    /// A pending prompt timed out and was auto-declined.
    Expired,
    Closed,
}
