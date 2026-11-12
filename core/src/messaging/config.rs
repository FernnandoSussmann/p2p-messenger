//! Every engine limit, as a named constant, and the config that carries them.

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
use std::time::Duration;

pub const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Slow-sender bound per inbound stream.
pub const FRAME_READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Doubled per counted failure.
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Counted attempts per session before a message becomes Failed.
pub const MAX_ATTEMPTS: u32 = 5;
/// Undelivered messages, all peers.
pub const MAX_OUTBOX: usize = 500;
/// Concurrent unacked outbound streams per peer.
pub const MAX_IN_FLIGHT_PER_PEER: usize = 8;
/// Concurrent inbound streams awaiting persist, per peer.
pub const MAX_INBOUND_PER_PEER: usize = 16;
pub const EVENT_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct MessagingConfig {
    pub ack_timeout: Duration,
    pub frame_read_timeout: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub max_attempts: u32,
    pub max_outbox: usize,
    pub max_in_flight_per_peer: usize,
    pub max_inbound_per_peer: usize,
    #[cfg(any(test, feature = "test-support"))]
    pub faults: Arc<super::testing::Faults>,
}

impl MessagingConfig {
    pub fn desktop() -> Self {
        Self {
            ack_timeout: ACK_TIMEOUT,
            frame_read_timeout: FRAME_READ_TIMEOUT,
            backoff_base: BACKOFF_BASE,
            backoff_max: BACKOFF_MAX,
            max_attempts: MAX_ATTEMPTS,
            max_outbox: MAX_OUTBOX,
            max_in_flight_per_peer: MAX_IN_FLIGHT_PER_PEER,
            max_inbound_per_peer: MAX_INBOUND_PER_PEER,
            #[cfg(any(test, feature = "test-support"))]
            faults: Arc::default(),
        }
    }

    /// Short timeouts for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fast() -> Self {
        Self {
            ack_timeout: Duration::from_millis(300),
            frame_read_timeout: Duration::from_secs(2),
            backoff_base: Duration::from_millis(50),
            backoff_max: Duration::from_millis(200),
            max_attempts: 3,
            ..Self::desktop()
        }
    }

    /// Delay after the `attempts`-th counted failure.
    pub(super) fn backoff(&self, attempts: u32) -> Duration {
        let doubling = 1u32 << attempts.saturating_sub(1).min(16);
        self.backoff_base
            .saturating_mul(doubling)
            .min(self.backoff_max)
    }

    pub(super) fn record_outbound(&self, in_flight: usize) {
        #[cfg(any(test, feature = "test-support"))]
        self.faults.record_outbound(in_flight);
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = in_flight;
    }

    pub(super) fn record_inbound(&self, live: usize) {
        #[cfg(any(test, feature = "test-support"))]
        self.faults.record_inbound(live);
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = live;
    }

    /// True when the next ack must be dropped (fault injection).
    pub(super) fn take_ack_drop(&self) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        return self.faults.take_ack_drop();
        #[cfg(not(any(test, feature = "test-support")))]
        false
    }

    pub(super) fn count_ack(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.faults.count_ack();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let c = MessagingConfig::desktop();
        assert_eq!(c.backoff(1), Duration::from_secs(1));
        assert_eq!(c.backoff(2), Duration::from_secs(2));
        assert_eq!(c.backoff(4), Duration::from_secs(8));
        assert_eq!(c.backoff(6), BACKOFF_MAX);
        assert_eq!(c.backoff(u32::MAX), BACKOFF_MAX);
    }

    #[test]
    fn desktop_uses_the_named_constants() {
        let c = MessagingConfig::desktop();
        assert_eq!(c.ack_timeout, ACK_TIMEOUT);
        assert_eq!(c.max_attempts, MAX_ATTEMPTS);
        assert_eq!(c.max_outbox, MAX_OUTBOX);
        assert_eq!(c.max_in_flight_per_peer, MAX_IN_FLIGHT_PER_PEER);
        assert_eq!(c.max_inbound_per_peer, MAX_INBOUND_PER_PEER);
    }
}
