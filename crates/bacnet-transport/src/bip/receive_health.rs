//! Local readiness of the sockets a B/IP receive loop reads.

use bacnet_types::sync::MutexExt as _;
use std::sync::{Arc, Mutex, Weak};

use super::BipTransport;

/// The receive sockets of one transport at the instant of the read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BipReceiveHealthSnapshot {
    /// Whether the primary socket has a live receive loop.
    pub primary_receive_active: bool,
    /// Subnet broadcast sockets required by per-address sharing on this platform.
    pub broadcast_listeners_required: usize,
    /// Open subnet broadcast sockets, including any optional multicast-mode sockets.
    pub broadcast_listeners_active: usize,
    /// Whether the loop's current multicast configuration uses a separate group socket.
    pub group_listener_required: bool,
    /// Whether the loop can still read that separate group socket.
    pub group_listener_active: bool,
}

impl BipReceiveHealthSnapshot {
    /// Whether the primary and all required listener sockets are active.
    pub fn ready(&self) -> bool {
        self.primary_receive_active
            && self.broadcast_listeners_active >= self.broadcast_listeners_required
            && (!self.group_listener_required || self.group_listener_active)
    }
}

/// A handle to a B/IP transport's current receive loop, including after a restart.
#[derive(Clone, Default)]
pub struct BipReceiveHealth {
    current: Arc<Mutex<Weak<Mutex<BipReceiveHealthSnapshot>>>>,
}

impl BipReceiveHealth {
    /// Read the current socket state without sending a packet.
    pub fn snapshot(&self) -> BipReceiveHealthSnapshot {
        self.current
            .locked()
            .upgrade()
            .map_or_else(BipReceiveHealthSnapshot::default, |run| *run.locked())
    }

    pub(super) fn stop(&self) {
        *self.current.locked() = Weak::new();
    }
}

impl BipTransport {
    /// Observe this transport's receive sockets across starts and stops.
    pub fn receive_health(&self) -> BipReceiveHealth {
        self.receive_health.clone()
    }
}

/// Owned by the listeners in the receive task. Dropping them retires this run,
/// even when its future was aborted before it was first polled.
#[derive(Default)]
pub(super) struct ReceiveRun(Arc<Mutex<BipReceiveHealthSnapshot>>);

impl ReceiveRun {
    pub(super) fn publish(
        &self,
        monitor: &BipReceiveHealth,
        required: bool,
        count: usize,
        group: bool,
    ) {
        *self.0.locked() = BipReceiveHealthSnapshot {
            primary_receive_active: true,
            broadcast_listeners_required: if required { count } else { 0 },
            broadcast_listeners_active: count,
            group_listener_required: group,
            group_listener_active: group,
        };
        *monitor.current.locked() = Arc::downgrade(&self.0);
    }

    pub(super) fn closed(&self, subnet_count: usize, group: bool) {
        let mut state = self.0.locked();
        state.broadcast_listeners_active = subnet_count;
        state.group_listener_active = group;
    }

    pub(super) fn replace_group(&self, active: bool) {
        let mut state = self.0.locked();
        state.group_listener_required = active;
        state.group_listener_active = active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::TransportPort;
    use std::net::Ipv4Addr;

    #[tokio::test(start_paused = true)]
    async fn a_monitor_follows_transport_start_and_stop() {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        let monitor = transport.receive_health();
        assert!(!monitor.snapshot().ready());
        let _rx = transport.start().await.unwrap();
        assert!(monitor.snapshot().ready());
        transport.stop().await.unwrap();
        assert_eq!(monitor.snapshot(), Default::default());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(start_paused = true)]
    async fn required_broadcast_bind_failure_does_not_publish_a_ready_run() {
        use crate::port_retry::{bind_error, rerun_on_lost_port};
        rerun_on_lost_port(async || {
            let held = std::net::UdpSocket::bind((Ipv4Addr::BROADCAST, 0))?;
            let port = held.local_addr()?.port();
            let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST);
            transport.set_share_port_by_address(true);
            let monitor = transport.receive_health();
            let err = transport.start().await.unwrap_err();
            if !err.to_string().contains("broadcast listener") {
                return Err(bind_error(err));
            }
            assert_eq!(monitor.snapshot(), Default::default());
            drop(held);
            let _rx = transport.start().await.map_err(bind_error)?;
            let running = monitor.snapshot();
            assert!(running.ready());
            assert!(running.broadcast_listeners_required > 0);
            transport.stop().await.unwrap();
            Ok(())
        })
        .await;
    }
}
