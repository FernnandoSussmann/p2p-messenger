//! Inbound-flood guard for the Accept/Decline prompt.
//!
//! A pure state machine: the clock is injected and nothing here does I/O, so
//! limits are unit-testable. Drops are meant to be silent.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::identity::NodeId;

/// Limits applied to unauthenticated inbound connections.
#[derive(Debug, Clone)]
pub struct GateLimits {
    /// Concurrent Accept/Decline prompts, global.
    pub max_pending: usize,
    /// Inbound attempts per NodeId per `attempt_window`.
    pub max_attempts_per_peer: u32,
    pub attempt_window: Duration,
    /// A pending prompt auto-declines after this long.
    pub prompt_ttl: Duration,
    /// Time allowed to read the first frame (`Hello`).
    pub hello_timeout: Duration,
    /// Concurrent pre-`Hello` connections.
    pub max_handshakes: usize,
    /// Size of the attempt table; the oldest entries are evicted beyond it.
    pub max_tracked_peers: usize,
}

impl Default for GateLimits {
    fn default() -> Self {
        Self {
            max_pending: 5,
            max_attempts_per_peer: 3,
            attempt_window: Duration::from_secs(60),
            prompt_ttl: Duration::from_secs(120),
            hello_timeout: Duration::from_secs(10),
            max_handshakes: 32,
            max_tracked_peers: 1024,
        }
    }
}

/// Outcome of [`FloodGuard::admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Prompt,
    Drop,
}

#[derive(Debug)]
pub struct FloodGuard {
    limits: GateLimits,
    pending: HashMap<NodeId, Instant>,
    attempts: HashMap<NodeId, VecDeque<Instant>>,
}

impl FloodGuard {
    pub fn new(limits: GateLimits) -> Self {
        Self {
            limits,
            pending: HashMap::new(),
            attempts: HashMap::new(),
        }
    }

    /// Decides whether an inbound attempt from `peer` may raise a prompt.
    /// Every call counts as an attempt, including ones that are dropped.
    pub fn admit(&mut self, peer: NodeId, now: Instant) -> Admission {
        let window = self.limits.attempt_window;
        let attempts = self.attempts.entry(peer).or_default();
        while attempts
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= window)
        {
            attempts.pop_front();
        }
        attempts.push_back(now);
        let too_many = attempts.len() > self.limits.max_attempts_per_peer as usize;

        self.evict_oldest_attempts();

        if too_many
            || self.pending.contains_key(&peer)
            || self.pending.len() >= self.limits.max_pending
        {
            return Admission::Drop;
        }
        self.pending.insert(peer, now);
        Admission::Prompt
    }

    /// Frees the prompt slot of `peer` once it was answered or closed.
    pub fn resolve(&mut self, peer: &NodeId) {
        self.pending.remove(peer);
    }

    /// Removes and returns the prompts that outlived `prompt_ttl`.
    pub fn expire(&mut self, now: Instant) -> Vec<NodeId> {
        let ttl = self.limits.prompt_ttl;
        let stale: Vec<NodeId> = self
            .pending
            .iter()
            .filter(|(_, t)| now.saturating_duration_since(**t) >= ttl)
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            self.pending.remove(id);
        }
        stale
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn evict_oldest_attempts(&mut self) {
        while self.attempts.len() > self.limits.max_tracked_peers {
            let oldest = self
                .attempts
                .iter()
                .filter(|(id, _)| !self.pending.contains_key(*id))
                .min_by_key(|(_, q)| q.back().copied())
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => {
                    self.attempts.remove(&id);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u32) -> NodeId {
        let mut seed = [0u8; 32];
        seed[..4].copy_from_slice(&n.to_le_bytes());
        iroh_base::SecretKey::from_bytes(&seed).public()
    }

    fn guard(limits: GateLimits) -> FloodGuard {
        FloodGuard::new(limits)
    }

    #[test]
    fn one_pending_prompt_per_peer() {
        let mut g = guard(GateLimits::default());
        let t = Instant::now();
        assert_eq!(g.admit(peer(1), t), Admission::Prompt);
        assert_eq!(g.admit(peer(1), t), Admission::Drop);
        assert_eq!(g.pending_len(), 1);
    }

    #[test]
    fn global_pending_cap_holds() {
        let mut g = guard(GateLimits::default());
        let t = Instant::now();
        for n in 0..5 {
            assert_eq!(g.admit(peer(n), t), Admission::Prompt);
        }
        assert_eq!(g.admit(peer(99), t), Admission::Drop);
        assert_eq!(g.pending_len(), 5);
    }

    #[test]
    fn resolve_frees_a_slot() {
        let mut g = guard(GateLimits {
            max_pending: 1,
            ..GateLimits::default()
        });
        let t = Instant::now();
        assert_eq!(g.admit(peer(1), t), Admission::Prompt);
        assert_eq!(g.admit(peer(2), t), Admission::Drop);
        g.resolve(&peer(1));
        assert_eq!(g.pending_len(), 0);
        assert_eq!(g.admit(peer(2), t), Admission::Prompt);
    }

    #[test]
    fn attempts_per_peer_are_limited_and_the_window_slides() {
        let mut g = guard(GateLimits::default());
        let t0 = Instant::now();
        for i in 0..3 {
            let now = t0 + Duration::from_secs(i);
            assert_eq!(g.admit(peer(1), now), Admission::Prompt);
            g.resolve(&peer(1));
        }
        let now = t0 + Duration::from_secs(3);
        assert_eq!(g.admit(peer(1), now), Admission::Drop);
        // Past the window of the first attempts, the peer may try again.
        let later = t0 + Duration::from_secs(61 + 3);
        assert_eq!(g.admit(peer(1), later), Admission::Prompt);
    }

    #[test]
    fn expire_returns_stale_prompts_only() {
        let mut g = guard(GateLimits::default());
        let t0 = Instant::now();
        g.admit(peer(1), t0);
        g.admit(peer(2), t0 + Duration::from_secs(100));
        let stale = g.expire(t0 + Duration::from_secs(121));
        assert_eq!(stale, vec![peer(1)]);
        assert_eq!(g.pending_len(), 1);
        assert!(g.expire(t0 + Duration::from_secs(121)).is_empty());
    }

    #[test]
    fn tracked_peer_table_stays_bounded() {
        let mut g = guard(GateLimits {
            max_tracked_peers: 10,
            max_pending: 0,
            ..GateLimits::default()
        });
        let t0 = Instant::now();
        for n in 0..100 {
            g.admit(peer(n), t0 + Duration::from_millis(n as u64));
        }
        assert!(g.attempts.len() <= 10);
        // The most recent peers are the ones still tracked.
        assert!(g.attempts.contains_key(&peer(99)));
        assert!(!g.attempts.contains_key(&peer(0)));
    }
}
