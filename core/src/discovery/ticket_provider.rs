//! The live [`TicketProvider`]: once a node is running, tickets come from its
//! endpoint's current addresses; before that, the NodeId-only fallback.

use std::sync::{Arc, RwLock};

use iroh::Endpoint;

use crate::identity::{NodeId, NodeIdTicketProvider, Ticket, TicketError, TicketProvider};

/// A cloneable handle shared between the identity service and the node.
#[derive(Clone, Default)]
pub struct LiveTicketProvider {
    endpoint: Arc<RwLock<Option<Endpoint>>>,
}

impl LiveTicketProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn attach(&self, endpoint: Endpoint) {
        if let Ok(mut slot) = self.endpoint.write() {
            *slot = Some(endpoint);
        }
    }

    pub(crate) fn detach(&self) {
        if let Ok(mut slot) = self.endpoint.write() {
            *slot = None;
        }
    }
}

impl TicketProvider for LiveTicketProvider {
    fn current_ticket(&self, node_id: &NodeId) -> Result<Ticket, TicketError> {
        let endpoint = self.endpoint.read().ok().and_then(|slot| slot.clone());
        match endpoint {
            Some(endpoint) => Ticket::from_endpoint_addr(&endpoint.addr()),
            None => NodeIdTicketProvider.current_ticket(node_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_provider_gives_the_node_id_only_ticket() {
        let id = iroh_base::SecretKey::from_bytes(&[5; 32]).public();
        let ticket = LiveTicketProvider::new().current_ticket(&id).unwrap();
        assert_eq!(ticket.as_str(), format!("p2pm:{id}"));
    }
}
