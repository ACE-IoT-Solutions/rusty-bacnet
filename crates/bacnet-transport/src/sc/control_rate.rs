//! Per-source rate limit for network-layer messages at SC NPDU ingress (#518).
//!
//! Routing-control messages (I-Am-Router-To-Network, Router-Busy and the
//! rest of Clause 6.4) can change a router's table and are relayed to its
//! other ports, so one peer sending them fast costs every port. The queue
//! quota in `npdu_admission` bounds how many NPDUs one source holds at once;
//! this bounds how many network-layer messages one source may hand in over
//! time.
//!
//! Each source (ingress path and source VMAC) has a token bucket: `burst`
//! messages at once, refilling at `per_second`. A message over budget is
//! dropped before it takes a queue slot. Only network-layer messages are
//! charged; APDUs, unicast or broadcast, never touch a bucket, so a control
//! flood can't starve application traffic from that source or any other.
//!
//! The source VMAC is a scheduling key, not an identity: a hub-relayed VMAC
//! is the hub's registration of its client, and a direct peer's is the VMAC
//! it claimed on its own connection.
//!
//! Bounds: at most [`CONTROL_RATE_SOURCES`] buckets. When the table is full,
//! buckets that have refilled completely are dropped first (losing nothing,
//! as a new bucket starts full). If none has, a new source shares one
//! overflow bucket, so sources already tracked keep their own budgets
//! however many new VMACs appear.

use std::collections::HashMap;

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use tokio::time::Instant;

use super::npdu_admission::ScIngressPath;

/// Most sources with their own control-message bucket per SC NPDU queue.
pub(crate) const CONTROL_RATE_SOURCES: usize = 256;

/// Per-source limit on inbound network-layer messages for one SC NPDU queue.
///
/// The default allows a burst of 32 and 8 a second after it, well above what
/// a router's start-up announcements and discovery need, and far below a
/// flood. Both values must be at least 1, and the burst at most 4,096.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScControlRateLimit {
    /// Messages one source may send back to back.
    pub burst: u32,
    /// Messages one source may send each second once its burst is spent.
    pub per_second: u32,
}

impl Default for ScControlRateLimit {
    fn default() -> Self {
        Self {
            burst: 32,
            per_second: 8,
        }
    }
}

impl ScControlRateLimit {
    /// Check the limit before any transport I/O or state change.
    pub fn validate(&self) -> Result<(), Error> {
        if self.burst == 0 || self.burst > 4096 || self.per_second == 0 {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC control-message rate needs a burst in 1..=4096 and a \
                 nonzero rate, got burst {} at {}/s",
                self.burst, self.per_second
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    refilled: Instant,
}

impl Bucket {
    fn full(limit: ScControlRateLimit, now: Instant) -> Self {
        Self {
            tokens: f64::from(limit.burst),
            refilled: now,
        }
    }

    fn refill(&mut self, limit: ScControlRateLimit, now: Instant) {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * f64::from(limit.per_second)).min(f64::from(limit.burst));
        self.refilled = now;
    }

    fn take(&mut self, limit: ScControlRateLimit, now: Instant) -> bool {
        self.refill(limit, now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_full(&self, limit: ScControlRateLimit, now: Instant) -> bool {
        let mut probe = *self;
        probe.refill(limit, now);
        probe.tokens >= f64::from(limit.burst)
    }
}

/// The buckets of one SC NPDU queue. Guarded by the admission lock.
#[derive(Debug)]
pub(crate) struct ControlRate {
    limit: ScControlRateLimit,
    sources: HashMap<(ScIngressPath, MacAddr), Bucket>,
    overflow: Option<Bucket>,
}

impl ControlRate {
    pub(crate) fn new(limit: ScControlRateLimit) -> Self {
        Self {
            limit,
            sources: HashMap::new(),
            overflow: None,
        }
    }

    /// Whether `source` may hand in one more network-layer message now,
    /// charging its bucket if so.
    pub(crate) fn admit(&mut self, path: ScIngressPath, source: &MacAddr, now: Instant) -> bool {
        let limit = self.limit;
        let key = (path, source.clone());
        if let Some(bucket) = self.sources.get_mut(&key) {
            return bucket.take(limit, now);
        }
        if self.sources.len() >= CONTROL_RATE_SOURCES {
            self.sources.retain(|_, bucket| !bucket.is_full(limit, now));
        }
        if self.sources.len() < CONTROL_RATE_SOURCES {
            let mut bucket = Bucket::full(limit, now);
            let admitted = bucket.take(limit, now);
            self.sources.insert(key, bucket);
            return admitted;
        }
        self.overflow
            .get_or_insert_with(|| Bucket::full(limit, now))
            .take(limit, now)
    }

    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.sources.len()
    }
}

/// Whether `npdu` carries a network-layer message rather than an APDU: an
/// ASHRAE NPDU (version 1) with the control octet's message bit set
/// (Clause 6.2.2). Anything shorter or of another version is left to the
/// network layer, which refuses it there.
pub(crate) fn is_network_message(npdu: &[u8]) -> bool {
    matches!(npdu, [0x01, control, ..] if control & 0x80 != 0)
}

#[cfg(test)]
#[path = "control_rate_tests.rs"]
mod tests;
