//! The `discovery` slice: the live iroh endpoint, tickets, the NodeId-based
//! trust gate and connection events.
//!
//! Trust is keyed on the transport-verified NodeId only; the display name a
//! peer announces in its `Hello` is advisory and never used as a key. Unknown
//! peers are held behind an Accept/Decline prompt and get no traffic past
//! `Hello` until accepted. LAN discovery (mDNS) only lists candidates: picking
//! one goes through the same ticket dial and trust gate. [`event_json`] is the single mapping from a core
//! event to its external name and JSON payload.

mod dto;
mod events;
mod flood;
mod hello;
mod known_peers;
mod lan;
mod node;
mod ticket_provider;

pub use dto::{
    event_json, ConnectionStateDto, IncomingRequestDto, LanPeerDto, MyTicketDto, PresenceDto,
};
pub use events::{ConnectionState, Direction, FailureReason, IncomingRequest, NetEvent};
pub use flood::{Admission, FloodGuard, GateLimits};
pub use known_peers::{FileKnownPeers, KnownPeers, KnownPeersError};
pub use lan::{
    decode_lan_name, encode_lan_name, LanDiscovery, LanError, LanEvent, LanMode, LanPeer, LanPeers,
    MdnsLan, LAN_NAME_MAX_BYTES, MAX_LAN_PEERS,
};
pub use node::{
    ConnectError, Node, NodeConfig, NodeDeps, PresenceError, RelayChoice, RespondError, StartError,
    CLOSE_BUSY, CLOSE_DECLINED, CLOSE_PROTOCOL,
};
pub use ticket_provider::LiveTicketProvider;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
