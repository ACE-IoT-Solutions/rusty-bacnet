//! The `bacnet-router-core` engine as the hosted router runs it (#1679).
//!
//! One [`Engine`] per router holds the core's [`Router`] over boxed storage,
//! behind a std mutex. Every port's dispatch, the sweep task and the table
//! guard take that lock for one synchronous call and the drain of what the
//! call asks to send, and none of them holds it across an await. The
//! engine's hooks (group rules, the control gate and its authorizer, the
//! logs) run under it too, so none of them may block or call back into the
//! router. A panic can't leave the engine half-changed between calls, so a
//! poisoned lock is taken as it is.
//!
//! The engine runs on Tokio time from one origin, taken when the router
//! starts ([`RouterClock`]), so paused-time tests drive its timers as they
//! drive the transports'.

use std::sync::{Mutex, MutexGuard, PoisonError};

use bacnet_router_core::router::{ConfigError, Limits, Options, PortConfig};
use bacnet_router_core::{Duration, Instant, Router, Storage};

use crate::router_table::{HOLD_RECORDS, ROUTE_CAP};

/// The most ports a router has: a routing table's Port ID names 255.
pub(super) const PORTS: usize = 255;

/// The unknown networks the router looks for at once (Clause 6.6.3.1).
pub(super) const SEARCHES: usize = 256;

/// The output scratch: the longest NPDU a port carries (BACnet/SC carries
/// up to 65,535 octets between hosts, Clause 6.6), plus the SNET, SLEN and
/// longest SADR a rewrite adds.
pub(super) const SCRATCH: usize = 65_535 + 2 + 1 + bacnet_router_core::MAX_MAC_LEN;

/// What a hosted router's engine keeps, boxed: about 115 KB.
type HostedStorage = Storage<PORTS, ROUTE_CAP, HOLD_RECORDS, SEARCHES, SCRATCH>;

/// The engine as the hosted router runs it.
pub(super) type HostedRouter =
    Router<Box<HostedStorage>, PORTS, ROUTE_CAP, HOLD_RECORDS, SEARCHES, SCRATCH>;

/// The hosted router's options: today's policy values and timers, the
/// 256-route cap, and none of the Clause 6 fixes yet.
const OPTIONS: Options = Options::HOSTED;

/// Each port's NPDU limits. With no fixes on, the engine checks none, and
/// the scratch is the only bound.
const LIMITS: Limits = Limits::both(u16::MAX);

/// Tokio time on the engine's nanosecond timeline, from the router's start.
#[derive(Debug, Clone, Copy)]
pub(super) struct RouterClock {
    origin: tokio::time::Instant,
}

impl RouterClock {
    /// A clock whose zero is now.
    pub(super) fn start() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> Instant {
        let elapsed = tokio::time::Instant::now().saturating_duration_since(self.origin);
        // Saturates some 584 years after the router started.
        Instant::ZERO + Duration::saturating_from_core(elapsed)
    }

    /// The Tokio instant of an engine time. Both ways are exact, so a timer
    /// set here never wakes before the engine's deadline is due.
    pub(super) fn instant(&self, at: Instant) -> tokio::time::Instant {
        let offset = std::time::Duration::from_nanos(at.as_nanos());
        self.origin
            .checked_add(offset)
            .unwrap_or_else(|| self.origin + std::time::Duration::from_secs(86_400 * 365))
    }
}

/// The engine of one router, with its clock.
pub(super) struct Engine {
    router: Mutex<HostedRouter>,
    clock: RouterClock,
}

impl Engine {
    /// An engine on `ports`, in port order, started on `clock`: each port's
    /// network installed as a direct route and the first sweep a period
    /// away. Refused as the core refuses: no port, more than 255, two on
    /// one network, or a router MAC longer than 18 octets.
    pub(super) fn new(ports: &[PortConfig], clock: RouterClock) -> Result<Self, ConfigError> {
        let router = Router::new(boxed_storage(), ports, OPTIONS, clock.now())?;
        Ok(Self {
            router: Mutex::new(router),
            clock,
        })
    }

    /// A port on `network` with the router's `mac` on it.
    pub(super) fn port(network: u16, mac: bacnet_router_core::Mac) -> PortConfig {
        PortConfig {
            network,
            mac,
            max_npdu: LIMITS,
        }
    }

    /// The engine, locked. Hold the guard for one call and its drain, never
    /// across an await.
    pub(super) fn lock(&self) -> MutexGuard<'_, HostedRouter> {
        self.router.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether someone holds the engine's lock now.
    #[cfg(test)]
    pub(super) fn is_locked(&self) -> bool {
        matches!(
            self.router.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        )
    }

    /// The engine's clock.
    pub(super) fn clock(&self) -> RouterClock {
        self.clock
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> Instant {
        self.clock.now()
    }
}

/// Zeroed storage on the heap. Built here, outside any async function, so
/// the 115 KB never sit in a future's state.
#[inline(never)]
fn boxed_storage() -> Box<HostedStorage> {
    Box::new(Storage::new())
}
