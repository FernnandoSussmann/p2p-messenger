//! The trust seam the gate consults. The production implementation is
//! `contacts::ContactBook`. The node never creates trust on **inbound**
//! traffic; outbound dials are remembered via `remember_dialed`.

use crate::identity::{DisplayName, NodeId};

/// Which NodeIds may connect without an Accept/Decline prompt.
pub trait KnownPeers: Send + Sync + 'static {
    /// Trust check, keyed on NodeId only. An `Err` must be treated as
    /// "unknown" (fail closed): the caller prompts instead of connecting
    /// silently.
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError>;

    /// Advisory name of an already-known peer. Hello (authorized connection):
    /// `name` replaces the stored name (None clears) and marks it seen.
    /// Lan: called only with Some(name); never marks "seen". Must never create
    /// an entry or change trust; a no-op for unknown ids.
    fn observe(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
        source: NameSource,
    ) -> Result<(), KnownPeersError>;

    /// A peer we deliberately dialed replied with a valid Hello (DD-03, user
    /// decision E): remember it as an Unverified contact with the default
    /// nickname if it is not one yet.
    fn remember_dialed(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
    ) -> Result<(), KnownPeersError>;
}

/// Where an advisory name was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    Hello,
    Lan,
}

/// Opaque: the gate only needs "failed" (fail closed).
#[derive(Debug)]
pub struct KnownPeersError;

impl std::fmt::Display for KnownPeersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("known peers store failed")
    }
}

impl std::error::Error for KnownPeersError {}
