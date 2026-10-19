//! A foreign device's registration target, which the Network Port object
//! reads and changes: FD_BBMD_Address and FD_Subscription_Lifetime (Clause
//! 12.56.37, 12.56.38, #1825).
//!
//! The transport, the object's live view and the running node share one
//! [`ForeignTarget`]. A written BBMD or time-to-live is checked when it is
//! written and again before a ReinitializeDevice handler runs; once
//! activated, the running node registers with the new BBMD from then on
//! ([`NodeHandle::retarget`]), and a later start uses the new values too.
//!
//! Lock order: the object database (held by the caller), then this
//! target's lock, then the node's. No lock here is held across an await.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::foreign_device::ForeignDeviceConfig;
use super::node::NodeHandle;

/// What the target's lock guards.
struct TargetState {
    /// The registration in effect, and the one the next start uses.
    config: ForeignDeviceConfig,
    /// The running node, while one runs.
    node: Weak<NodeHandle>,
}

/// One transport's foreign-device registration, shared with the Network
/// Port object it is lent to.
pub(crate) struct ForeignTarget {
    state: Mutex<TargetState>,
    /// The transport's broadcast address and interface, against which a new
    /// BBMD is checked as at start ([`ForeignDeviceConfig::check_bbmd`]).
    broadcast: Ipv4Addr,
    interface: Ipv4Addr,
}

/// The WriteProperty refusal of a value the foreign device can't register
/// with.
fn out_of_range() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::VALUE_OUT_OF_RANGE,
    }
}

impl ForeignTarget {
    pub(super) fn new(
        config: ForeignDeviceConfig,
        broadcast: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Self {
        Self {
            state: Mutex::new(TargetState {
                config,
                node: Weak::new(),
            }),
            broadcast,
            interface,
        }
    }

    fn lock(&self) -> MutexGuard<'_, TargetState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registration in effect.
    pub(super) fn config(&self) -> ForeignDeviceConfig {
        self.lock().config.clone()
    }

    /// Change the configuration in a test, as a caller with the transport
    /// could before the target existed.
    #[cfg(test)]
    pub(super) fn edit(&self, edit: impl FnOnce(&mut ForeignDeviceConfig)) {
        edit(&mut self.lock().config);
    }

    /// Hand the target the node a start built from `used`. A registration
    /// activated while the start ran goes to the node now, so it never runs
    /// on values the object no longer reports.
    pub(super) fn attach(
        &self,
        node: &Arc<NodeHandle>,
        used: &ForeignDeviceConfig,
    ) -> Result<(), Error> {
        let mut state = self.lock();
        if state.config != *used {
            let interval = state.config.interval()?;
            node.retarget(state.config.engine(interval))?;
        }
        state.node = Arc::downgrade(node);
        Ok(())
    }

    /// The registration `changes` would make, checked as a start checks
    /// one, and against the running node's own address. Changes nothing.
    fn changed(
        &self,
        state: &TargetState,
        changes: &ForeignDeviceChanges,
    ) -> Result<ForeignDeviceConfig, Error> {
        let mut config = state.config.clone();
        if let Some(bbmd) = &changes.bbmd {
            // A B/IP address: an IPv4 host and a port. This device resolves
            // no host names, as for the BDT rows (#1578), and 'none' names
            // no BBMD.
            let BACnetHostAddress::Ip(IpAddr::V4(ip)) = bbmd.host else {
                return Err(out_of_range());
            };
            config.bbmd_ip = ip;
            config.bbmd_port = bbmd.port;
        }
        if let Some(ttl) = changes.subscription_lifetime {
            config.ttl = ttl;
        }
        // A zero time-to-live, or one the configured renewal interval
        // doesn't fit under, is refused as at start.
        let interval = config.interval().map_err(|_| out_of_range())?;
        config
            .check_bbmd(self.broadcast, self.interface)
            .map_err(|_| out_of_range())?;
        if let Some(node) = state.node.upgrade() {
            node.check_retarget(config.engine(interval))
                .map_err(|_| out_of_range())?;
        }
        Ok(config)
    }
}

impl ForeignRegistration for ForeignTarget {
    fn bbmd(&self) -> BACnetHostNPort {
        let config = &self.lock().config;
        BACnetHostNPort::from_socket_addr(SocketAddr::V4(SocketAddrV4::new(
            config.bbmd_ip,
            config.bbmd_port,
        )))
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let state = self.lock();
        self.changed(&state, changes).map(drop)
    }

    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        let config = self.changed(&state, changes)?;
        // With no node running, the next start takes the new values.
        if let Some(node) = state.node.upgrade() {
            let interval = config.interval().map_err(|_| out_of_range())?;
            node.retarget(config.engine(interval))
                .map_err(|_| out_of_range())?;
        }
        state.config = config;
        Ok(())
    }
}

impl super::BipTransport {
    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. `None` outside
    /// foreign-device mode.
    pub fn foreign_device_config(&self) -> Option<ForeignDeviceConfig> {
        self.foreign_device.as_ref().map(|target| target.config())
    }
}
