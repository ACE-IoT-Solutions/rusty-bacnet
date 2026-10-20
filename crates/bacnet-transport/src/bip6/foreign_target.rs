//! A B/IPv6 foreign device's registration target, which the Network Port
//! object reads and changes: FD_BBMD_Address and FD_Subscription_Lifetime
//! (Clause 12.56.37, 12.56.38; Table 12-71 footnote 13 applies them to
//! IPV6 ports), as B/IP's `ForeignTarget` does for B/IP (#1825, #1579).
//!
//! The transport, its receive loop, its resolution driver, its broadcasts,
//! its re-registration task and the object's live view all read one
//! [`ForeignTarget`], so an activated BBMD or time-to-live takes effect
//! everywhere at once: the re-registration task registers with the new
//! values straight away, and a later start uses them too. Only a running
//! transport takes a change, since only it knows its own bound address,
//! which a new BBMD must not be.
//!
//! Lock order: the object database (held by the caller), then this
//! target's lock. The lock is a short synchronous critical section, never
//! held across an await.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::Notify;

use super::port::Bip6ForeignDeviceConfig;

/// What the target's lock guards.
struct State {
    /// The registration in effect, and the one the next start uses.
    config: Bip6ForeignDeviceConfig,
    /// This node's bound address while the transport runs.
    own: Option<SocketAddrV6>,
}

/// One transport's foreign-device registration, shared with the Network
/// Port object it is lent to.
pub(crate) struct ForeignTarget {
    state: Mutex<State>,
    /// Wakes the re-registration task when an activation changes the
    /// registration.
    changed: Notify,
}

fn property_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code,
    }
}

impl ForeignTarget {
    pub(super) fn new(config: Bip6ForeignDeviceConfig) -> Self {
        Self {
            state: Mutex::new(State { config, own: None }),
            changed: Notify::new(),
        }
    }

    /// A target for `bbmd`, for a test's receive loop or driver.
    #[cfg(test)]
    pub(super) fn at(bbmd: SocketAddrV6) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::new(Bip6ForeignDeviceConfig {
            bbmd_ip: *bbmd.ip(),
            bbmd_port: bbmd.port(),
            ttl: 60,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registration in effect.
    pub(super) fn config(&self) -> Bip6ForeignDeviceConfig {
        self.lock().config.clone()
    }

    /// The BBMD's address and port, as the receive path checks a sender.
    pub(super) fn bbmd(&self) -> (Ipv6Addr, u16) {
        let state = self.lock();
        (state.config.bbmd_ip, state.config.bbmd_port)
    }

    /// The BBMD as a destination.
    pub(super) fn bbmd_addr(&self) -> SocketAddrV6 {
        let (ip, port) = self.bbmd();
        SocketAddrV6::new(ip, port, 0, 0)
    }

    /// How long until the next re-registration: half the time-to-live, and
    /// never under 30 s.
    pub(super) fn renewal_interval(&self) -> Duration {
        Duration::from_secs((u64::from(self.lock().config.ttl) / 2).max(30))
    }

    /// The transport runs, bound to `own`: changes are taken from now on.
    pub(super) fn attach(&self, own: SocketAddrV6) {
        self.lock().own = Some(own);
    }

    /// The transport stopped: changes are refused until it runs again.
    pub(super) fn detach(&self) {
        self.lock().own = None;
    }

    /// Resolve when an activation changed the registration.
    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    /// The registration `changes` would make, checked against this node's
    /// own address. Changes nothing.
    fn changed_config(
        state: &State,
        changes: &ForeignDeviceChanges,
    ) -> Result<Bip6ForeignDeviceConfig, Error> {
        let own = state
            .own
            .ok_or_else(|| property_error(ErrorCode::WRITE_ACCESS_DENIED))?;
        let out_of_range = || property_error(ErrorCode::VALUE_OUT_OF_RANGE);
        let mut config = state.config.clone();
        if let Some(bbmd) = &changes.bbmd {
            // A B/IPv6 address: an IPv6 host and a port. This device
            // resolves no host names, and 'none' names no BBMD.
            let BACnetHostAddress::Ip(IpAddr::V6(ip)) = bbmd.host else {
                return Err(out_of_range());
            };
            config.bbmd_ip = ip;
            config.bbmd_port = bbmd.port;
        }
        if let Some(ttl) = changes.subscription_lifetime {
            config.ttl = ttl;
        }
        let ip = config.bbmd_ip;
        // A link-local BBMD is never trusted as the sender of a
        // Forwarded-NPDU (no zone index travels with it), so it can't
        // serve as one.
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_unicast_link_local()
            || ip.to_ipv4_mapped().is_some()
            || config.bbmd_port == 0
            || config.ttl == 0
            || (ip == *own.ip() && config.bbmd_port == own.port())
        {
            return Err(out_of_range());
        }
        Ok(config)
    }
}

impl ForeignRegistration for ForeignTarget {
    fn bbmd(&self) -> BACnetHostNPort {
        BACnetHostNPort::from_socket_addr(SocketAddr::V6(self.bbmd_addr()))
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        Self::changed_config(&self.lock(), changes).map(drop)
    }

    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        state.config = Self::changed_config(&state, changes)?;
        drop(state);
        // One permit is stored if the task isn't waiting yet, so the change
        // is never missed.
        self.changed.notify_one();
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}
