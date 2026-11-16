//! The reviewed logging vocabulary: log calls use these helpers, plain enums
//! and counters, and nothing else.

use std::fmt;

use iroh::TransportAddr;

use crate::identity::NodeId;

/// Short NodeId: enough to correlate, less metadata than the full key.
pub fn peer(id: &NodeId) -> PeerField {
    PeerField(*id)
}

#[derive(Clone, Copy)]
pub struct PeerField(NodeId);

impl fmt::Display for PeerField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.fmt_short(), f)
    }
}

/// `relay:<url>`, `ip:<socketaddr>` or `custom`.
pub fn transport(addr: &TransportAddr) -> TransportField<'_> {
    TransportField(addr)
}

/// `direct`, `relay` or `custom`.
pub fn path_kind(addr: &TransportAddr) -> &'static str {
    match addr {
        TransportAddr::Ip(_) => "direct",
        TransportAddr::Relay(_) => "relay",
        _ => "custom",
    }
}

#[derive(Clone, Copy)]
pub struct TransportField<'a>(&'a TransportAddr);

impl fmt::Display for TransportField<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            TransportAddr::Relay(url) => write!(f, "relay:{url}"),
            TransportAddr::Ip(addr) => write!(f, "ip:{addr}"),
            _ => f.write_str("custom"),
        }
    }
}
