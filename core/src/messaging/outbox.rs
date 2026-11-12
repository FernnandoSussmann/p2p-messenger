//! The bounded in-memory outbox: per-peer queues of undelivered messages.
//!
//! Invariant: `used` (entries plus reservations) never exceeds `max`, so the
//! persisted Sending/Pending rows never exceed it either.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use protocol::{ChatMessage, MessageId};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::MessageStatus;
use crate::identity::NodeId;

pub(super) struct Entry {
    pub msg: ChatMessage,
    pub attempts: u32,
    pub next_try: Instant,
    pub in_flight: bool,
    /// The last status persisted for this row.
    pub status: MessageStatus,
}

impl Entry {
    pub fn new(msg: ChatMessage, status: MessageStatus) -> Self {
        Self {
            msg,
            attempts: 0,
            next_try: Instant::now(),
            in_flight: false,
            status,
        }
    }
}

#[derive(Default)]
struct PeerQueue {
    entries: VecDeque<Entry>,
    driver_running: bool,
    wake: Arc<Notify>,
}

pub(super) struct Outbox {
    max: usize,
    used: usize,
    peers: HashMap<NodeId, PeerQueue>,
}

/// What a driver should do next, computed under the outbox lock.
pub(super) struct Plan {
    pub start: Vec<ChatMessage>,
    pub next_try: Option<Instant>,
    pub in_flight: usize,
}

impl Outbox {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            used: 0,
            peers: HashMap::new(),
        }
    }

    pub fn reserve(&mut self) -> bool {
        if self.used >= self.max {
            return false;
        }
        self.used += 1;
        true
    }

    pub fn release(&mut self) {
        self.used = self.used.saturating_sub(1);
    }

    pub fn contains(&self, peer: &NodeId, id: &MessageId) -> bool {
        self.peers
            .get(peer)
            .is_some_and(|q| q.entries.iter().any(|e| e.msg.id == *id))
    }

    /// Queues an entry. The caller holds a reservation for it.
    pub fn push(&mut self, peer: NodeId, entry: Entry) {
        self.peers.entry(peer).or_default().entries.push_back(entry);
    }

    /// Queues an entry from the restart reload, reserving its slot.
    pub fn push_reserving(&mut self, peer: NodeId, entry: Entry) -> bool {
        if !self.reserve() {
            return false;
        }
        self.push(peer, entry);
        true
    }

    pub fn wake_handle(&mut self, peer: NodeId) -> Arc<Notify> {
        self.peers.entry(peer).or_default().wake.clone()
    }

    pub fn notify(&self, peer: &NodeId) {
        if let Some(q) = self.peers.get(peer) {
            q.wake.notify_one();
        }
    }

    /// Marks a driver as running for `peer`; false if one already is or there
    /// is nothing to drive.
    pub fn claim_driver(&mut self, peer: &NodeId) -> bool {
        match self.peers.get_mut(peer) {
            Some(q) if !q.driver_running && !q.entries.is_empty() => {
                q.driver_running = true;
                true
            }
            _ => false,
        }
    }

    /// The driver found an empty queue: it exits and the queue is dropped.
    pub fn release_driver_if_empty(&mut self, peer: &NodeId) -> bool {
        let empty = self.peers.get(peer).is_none_or(|q| q.entries.is_empty());
        if empty {
            self.peers.remove(peer);
        }
        empty
    }

    pub fn status_of(&self, peer: &NodeId, id: &MessageId) -> Option<MessageStatus> {
        self.peers
            .get(peer)?
            .entries
            .iter()
            .find(|e| e.msg.id == *id)
            .map(|e| e.status)
    }

    pub fn update(&mut self, peer: &NodeId, id: &MessageId, f: impl FnOnce(&mut Entry)) -> bool {
        let found = self
            .peers
            .get_mut(peer)
            .and_then(|q| q.entries.iter_mut().find(|e| e.msg.id == *id));
        match found {
            Some(entry) => {
                f(entry);
                true
            }
            None => false,
        }
    }

    /// Removes an entry and frees its slot.
    pub fn remove(&mut self, peer: &NodeId, id: &MessageId) -> bool {
        let Some(q) = self.peers.get_mut(peer) else {
            return false;
        };
        let Some(i) = q.entries.iter().position(|e| e.msg.id == *id) else {
            return false;
        };
        q.entries.remove(i);
        self.used = self.used.saturating_sub(1);
        true
    }

    /// Removes every entry of `peer`, freeing their slots; returns the ids.
    pub fn take_all(&mut self, peer: &NodeId) -> Vec<MessageId> {
        let Some(q) = self.peers.get_mut(peer) else {
            return Vec::new();
        };
        let ids: Vec<MessageId> = q.entries.drain(..).map(|e| e.msg.id).collect();
        self.used = self.used.saturating_sub(ids.len());
        ids
    }

    /// Ids whose persisted status is `Sending`.
    pub fn sending_ids(&self, peer: &NodeId) -> Vec<MessageId> {
        self.peers.get(peer).map_or_else(Vec::new, |q| {
            q.entries
                .iter()
                .filter(|e| e.status == MessageStatus::Sending)
                .map(|e| e.msg.id)
                .collect()
        })
    }

    pub fn peers_with_entries(&self) -> Vec<NodeId> {
        self.peers
            .iter()
            .filter(|(_, q)| !q.entries.is_empty())
            .map(|(p, _)| *p)
            .collect()
    }

    /// Marks due entries in flight (queue order, up to the per-peer cap).
    pub fn plan(&mut self, peer: &NodeId, now: Instant, max_in_flight: usize) -> Plan {
        let mut plan = Plan {
            start: Vec::new(),
            next_try: None,
            in_flight: 0,
        };
        let Some(q) = self.peers.get_mut(peer) else {
            return plan;
        };
        plan.in_flight = q.entries.iter().filter(|e| e.in_flight).count();
        for e in q.entries.iter_mut().filter(|e| !e.in_flight) {
            if e.next_try <= now && plan.in_flight < max_in_flight {
                e.in_flight = true;
                plan.in_flight += 1;
                plan.start.push(e.msg.clone());
            } else if e.next_try > now {
                plan.next_try = Some(plan.next_try.map_or(e.next_try, |t| t.min(e.next_try)));
            }
        }
        plan
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use protocol::new_message_id;

    use super::*;

    fn entry(due: Instant) -> Entry {
        let mut e = Entry::new(
            ChatMessage {
                id: new_message_id(),
                sent_at_ms: 0,
                body: "x".into(),
            },
            MessageStatus::Pending,
        );
        e.next_try = due;
        e
    }

    fn peer() -> NodeId {
        iroh::SecretKey::generate().public()
    }

    #[test]
    fn reservations_are_bounded_and_released() {
        let mut o = Outbox::new(2);
        assert!(o.reserve() && o.reserve());
        assert!(!o.reserve());
        o.release();
        assert!(o.reserve());
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn remove_and_take_all_free_slots() {
        let (mut o, p) = (Outbox::new(3), peer());
        let now = Instant::now();
        for _ in 0..3 {
            assert!(o.push_reserving(p, entry(now)));
        }
        assert!(!o.push_reserving(p, entry(now)));
        let id = o.peers[&p].entries[0].msg.id;
        assert!(o.remove(&p, &id));
        assert!(!o.remove(&p, &id));
        assert_eq!(o.len(), 2);
        assert_eq!(o.take_all(&p).len(), 2);
        assert_eq!(o.len(), 0);
    }

    #[test]
    fn plan_respects_in_flight_cap_and_due_time() {
        let (mut o, p) = (Outbox::new(10), peer());
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        for due in [now, now, now, later] {
            assert!(o.push_reserving(p, entry(due)));
        }
        let plan = o.plan(&p, now, 2);
        assert_eq!(plan.start.len(), 2);
        assert_eq!(plan.in_flight, 2);
        assert_eq!(plan.next_try, Some(later));
        assert!(o.plan(&p, now, 2).start.is_empty());
    }

    #[test]
    fn driver_is_claimed_once_and_released_when_empty() {
        let (mut o, p) = (Outbox::new(2), peer());
        assert!(!o.claim_driver(&p));
        assert!(o.push_reserving(p, entry(Instant::now())));
        assert!(o.claim_driver(&p));
        assert!(!o.claim_driver(&p));
        assert!(!o.release_driver_if_empty(&p));
        o.take_all(&p);
        assert!(o.release_driver_if_empty(&p));
        assert!(o.push_reserving(p, entry(Instant::now())));
        assert!(o.claim_driver(&p));
    }
}
