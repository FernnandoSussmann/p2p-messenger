//! The `discovery` slice: the live iroh endpoint, tickets, the NodeId-based
//! trust gate and connection events.
//!
//! Trust is keyed on the transport-verified NodeId only; the display name a
//! peer announces in its `Hello` is advisory and never used as a key. Unknown
//! peers are held behind an Accept/Decline prompt and get no traffic past
//! `Hello` until accepted. [`event_json`] is the single mapping from a core
//! event to its external name and JSON payload.

mod dto;
mod events;
mod flood;
mod known_peers;

pub use dto::{event_json, ConnectionStateDto, IncomingRequestDto, MyTicketDto};
pub use events::{ConnectionState, Direction, FailureReason, IncomingRequest, NetEvent};
pub use flood::{Admission, FloodGuard, GateLimits};
pub use known_peers::{KnownPeers, KnownPeersError};

// ── track A modules ──

// ── track B modules ──
