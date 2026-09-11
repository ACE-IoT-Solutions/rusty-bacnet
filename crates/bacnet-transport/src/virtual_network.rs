//! Named, in-process BACnet virtual networks.
//!
//! A [`VirtualNetwork`] is a [`TransportPort`] whose one-byte MAC address is
//! registered in a process-wide, named hub. Networks with different names are
//! isolated. Unicast is delivered to exactly one matching member, while
//! broadcast is offered once to every other member and is never looped back to
//! the sender.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::port::{ReceivedNpdu, TransportPort};

/// Default number of queued NPDUs per virtual-network member.
pub const VIRTUAL_NETWORK_QUEUE_CAPACITY: usize = 256;

/// Maximum configurable queue capacity for one virtual-network member.
pub const MAX_VIRTUAL_NETWORK_QUEUE_CAPACITY: usize = 65_536;

const STATE_JOINED: u8 = 0;
const STATE_STARTED: u8 = 1;
const STATE_STOPPED: u8 = 2;

#[derive(Clone)]
struct Member {
    generation: u64,
    tx: mpsc::Sender<ReceivedNpdu>,
}

#[derive(Default)]
struct Registry {
    networks: HashMap<String, HashMap<u8, Member>>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn lock_registry() -> std::sync::MutexGuard<'static, Registry> {
    // No user callback runs under this lock. Keep the registry usable after a
    // test or caller panic rather than poisoning every in-process network.
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn io_error(kind: io::ErrorKind, message: impl Into<String>) -> Error {
    Error::Transport(io::Error::new(kind, message.into()))
}

fn unregister(network_name: &str, mac: u8, generation: u64) {
    let mut registry = lock_registry();
    let remove_network = if let Some(network) = registry.networks.get_mut(network_name) {
        // Generation ownership prevents a stale handle from removing a newer
        // member that rejoined with the same network name and MAC.
        if network
            .get(&mac)
            .is_some_and(|member| member.generation == generation)
        {
            network.remove(&mac);
        }
        network.is_empty()
    } else {
        false
    };
    if remove_network {
        registry.networks.remove(network_name);
    }
}

/// Snapshot of delivery outcomes produced by one virtual-network member.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VirtualNetworkStats {
    /// Successfully delivered unicast NPDUs.
    pub unicast_delivered: u64,
    /// Successfully delivered broadcast copies (one count per recipient).
    pub broadcast_copies_delivered: u64,
    /// Unicasts dropped because the destination MAC was not registered.
    pub unknown_destination_drops: u64,
    /// Delivery attempts dropped because a recipient queue was full.
    pub full_member_drops: u64,
    /// Delivery attempts dropped because a recipient receiver was closed.
    pub closed_member_drops: u64,
}

#[derive(Default)]
struct Counters {
    unicast_delivered: AtomicU64,
    broadcast_copies_delivered: AtomicU64,
    unknown_destination_drops: AtomicU64,
    full_member_drops: AtomicU64,
    closed_member_drops: AtomicU64,
}

/// One member of a named, N-node, in-process BACnet virtual network.
///
/// Membership begins when [`VirtualNetwork::join`] succeeds. Call
/// [`TransportPort::start`] once to take the receive queue. Calling `stop` or
/// dropping this value unregisters the membership. A stopped instance is
/// terminal; create a new instance to rejoin.
pub struct VirtualNetwork {
    network_name: String,
    local_mac: MacAddr,
    mac: u8,
    generation: u64,
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    state: AtomicU8,
    counters: Counters,
}

impl VirtualNetwork {
    /// Join `network_name` using an explicit one-byte virtual MAC address.
    pub fn join(network_name: impl Into<String>, mac: u8) -> Result<Self, Error> {
        Self::join_with_capacity(network_name, mac, VIRTUAL_NETWORK_QUEUE_CAPACITY)
    }

    /// Join with an explicit bounded receive queue capacity.
    pub fn join_with_capacity(
        network_name: impl Into<String>,
        mac: u8,
        queue_capacity: usize,
    ) -> Result<Self, Error> {
        let network_name = network_name.into();
        if network_name.trim().is_empty() {
            return Err(Error::OutOfRange(
                "virtual network name must not be empty".to_string(),
            ));
        }
        if !(1..=MAX_VIRTUAL_NETWORK_QUEUE_CAPACITY).contains(&queue_capacity) {
            return Err(Error::OutOfRange(format!(
                "virtual network queue capacity must be 1..={MAX_VIRTUAL_NETWORK_QUEUE_CAPACITY}"
            )));
        }

        let (tx, rx) = mpsc::channel(queue_capacity);
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        let mut registry = lock_registry();
        let network = registry.networks.entry(network_name.clone()).or_default();
        if network.contains_key(&mac) {
            return Err(io_error(
                io::ErrorKind::AddrInUse,
                format!("MAC {mac:#04x} is already joined to virtual network {network_name:?}"),
            ));
        }
        network.insert(mac, Member { generation, tx });
        drop(registry);

        Ok(Self {
            network_name,
            local_mac: MacAddr::from_slice(&[mac]),
            mac,
            generation,
            rx: Some(rx),
            state: AtomicU8::new(STATE_JOINED),
            counters: Counters::default(),
        })
    }

    /// The registry name isolating this virtual network from all others.
    pub fn network_name(&self) -> &str {
        &self.network_name
    }

    /// Return a consistent snapshot of this member's send outcomes.
    pub fn stats(&self) -> VirtualNetworkStats {
        VirtualNetworkStats {
            unicast_delivered: self.counters.unicast_delivered.load(Ordering::Relaxed),
            broadcast_copies_delivered: self
                .counters
                .broadcast_copies_delivered
                .load(Ordering::Relaxed),
            unknown_destination_drops: self
                .counters
                .unknown_destination_drops
                .load(Ordering::Relaxed),
            full_member_drops: self.counters.full_member_drops.load(Ordering::Relaxed),
            closed_member_drops: self.counters.closed_member_drops.load(Ordering::Relaxed),
        }
    }

    fn require_started(&self) -> Result<(), Error> {
        match self.state.load(Ordering::Acquire) {
            STATE_STARTED => Ok(()),
            STATE_STOPPED => Err(io_error(
                io::ErrorKind::NotConnected,
                "virtual network transport is stopped",
            )),
            _ => Err(io_error(
                io::ErrorKind::NotConnected,
                "virtual network transport has not been started",
            )),
        }
    }

    fn require_current_membership(&self, registry: &mut Registry) -> Result<(), Error> {
        let current = registry
            .networks
            .get(&self.network_name)
            .and_then(|network| network.get(&self.mac))
            .map(|member| (member.generation, member.tx.is_closed()));
        if current == Some((self.generation, false)) {
            return Ok(());
        }

        if current == Some((self.generation, true)) {
            let remove_network =
                if let Some(network) = registry.networks.get_mut(&self.network_name) {
                    if network
                        .get(&self.mac)
                        .is_some_and(|member| member.generation == self.generation)
                    {
                        network.remove(&self.mac);
                    }
                    network.is_empty()
                } else {
                    false
                };
            if remove_network {
                registry.networks.remove(&self.network_name);
            }
        }

        // Another sender may have pruned this generation after its receive
        // queue closed. Make the stale handle terminal without touching a
        // newer generation that may now own the same MAC.
        self.state.store(STATE_STOPPED, Ordering::Release);
        Err(io_error(
            io::ErrorKind::NotConnected,
            "virtual network membership is no longer current",
        ))
    }

    fn message(&self, npdu: &[u8], link_layer_group: bool) -> ReceivedNpdu {
        ReceivedNpdu {
            npdu: Bytes::copy_from_slice(npdu),
            source_mac: self.local_mac.clone(),
            link_layer_group,
            data_attributes: Vec::new(),
            transport_meta: None,
            reply_tx: None,
        }
    }
}

impl Drop for VirtualNetwork {
    fn drop(&mut self) {
        if self.state.swap(STATE_STOPPED, Ordering::AcqRel) != STATE_STOPPED {
            unregister(&self.network_name, self.mac, self.generation);
        }
    }
}

impl TransportPort for VirtualNetwork {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.state
            .compare_exchange(
                STATE_JOINED,
                STATE_STARTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|state| {
                if state == STATE_STOPPED {
                    io_error(
                        io::ErrorKind::NotConnected,
                        "virtual network transport is stopped",
                    )
                } else {
                    Error::Encoding("virtual network transport already started".to_string())
                }
            })?;
        self.rx.take().ok_or_else(|| {
            Error::Encoding("virtual network receive queue is unavailable".to_string())
        })
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if self.state.swap(STATE_STOPPED, Ordering::AcqRel) != STATE_STOPPED {
            unregister(&self.network_name, self.mac, self.generation);
        }
        self.rx.take();
        Ok(())
    }

    fn abort(&mut self) {
        if self.state.swap(STATE_STOPPED, Ordering::AcqRel) != STATE_STOPPED {
            unregister(&self.network_name, self.mac, self.generation);
        }
        self.rx.take();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.require_started()?;
        if mac.len() != 1 {
            return Err(Error::OutOfRange(
                "virtual network destination MAC must be exactly one byte".to_string(),
            ));
        }

        let destination = mac[0];
        let mut registry = lock_registry();
        self.require_current_membership(&mut registry)?;
        let Some(network) = registry.networks.get_mut(&self.network_name) else {
            self.counters
                .unknown_destination_drops
                .fetch_add(1, Ordering::Relaxed);
            return Err(io_error(
                io::ErrorKind::NotFound,
                "virtual network no longer exists",
            ));
        };
        let Some(member) = network.get(&destination).cloned() else {
            self.counters
                .unknown_destination_drops
                .fetch_add(1, Ordering::Relaxed);
            return Err(io_error(
                io::ErrorKind::NotFound,
                format!("virtual network destination MAC {destination:#04x} is not joined"),
            ));
        };

        match member.tx.try_send(self.message(npdu, false)) {
            Ok(()) => {
                self.counters
                    .unicast_delivered
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.counters
                    .full_member_drops
                    .fetch_add(1, Ordering::Relaxed);
                Err(io_error(
                    io::ErrorKind::WouldBlock,
                    format!("virtual network destination MAC {destination:#04x} queue is full"),
                ))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.counters
                    .closed_member_drops
                    .fetch_add(1, Ordering::Relaxed);
                if network
                    .get(&destination)
                    .is_some_and(|current| current.generation == member.generation)
                {
                    network.remove(&destination);
                }
                Err(io_error(
                    io::ErrorKind::NotConnected,
                    format!(
                        "virtual network destination MAC {destination:#04x} receiver is closed"
                    ),
                ))
            }
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.require_started()?;
        let mut registry = lock_registry();
        self.require_current_membership(&mut registry)?;
        let Some(network) = registry.networks.get_mut(&self.network_name) else {
            return Err(io_error(
                io::ErrorKind::NotConnected,
                "virtual network no longer exists",
            ));
        };

        let recipients: Vec<(u8, Member)> = network
            .iter()
            .filter(|(mac, _)| **mac != self.mac)
            .map(|(mac, member)| (*mac, member.clone()))
            .collect();
        let mut full = 0_u64;
        let mut closed = Vec::new();
        for (mac, member) in recipients {
            match member.tx.try_send(self.message(npdu, true)) {
                Ok(()) => {
                    self.counters
                        .broadcast_copies_delivered
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(mpsc::error::TrySendError::Full(_)) => full += 1,
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    closed.push((mac, member.generation));
                }
            }
        }
        self.counters
            .full_member_drops
            .fetch_add(full, Ordering::Relaxed);
        self.counters
            .closed_member_drops
            .fetch_add(closed.len() as u64, Ordering::Relaxed);
        for (mac, generation) in &closed {
            if network
                .get(mac)
                .is_some_and(|current| current.generation == *generation)
            {
                network.remove(mac);
            }
        }

        if full != 0 || !closed.is_empty() {
            let kind = if full != 0 {
                io::ErrorKind::WouldBlock
            } else {
                io::ErrorKind::NotConnected
            };
            return Err(io_error(
                kind,
                format!(
                    "virtual network broadcast dropped {full} full and {} closed recipient(s)",
                    closed.len()
                ),
            ));
        }
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn name(label: &str) -> String {
        format!(
            "virtual-network-test-{label}-{}",
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn io_kind(error: &Error) -> Option<io::ErrorKind> {
        match error {
            Error::Transport(error) => Some(error.kind()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn three_members_unicast_broadcast_and_exact_source() {
        let network = name("delivery");
        let mut a = VirtualNetwork::join(&network, 1).unwrap();
        let mut b = VirtualNetwork::join(&network, 2).unwrap();
        let mut c = VirtualNetwork::join(&network, 3).unwrap();
        let mut rx_a = a.start().await.unwrap();
        let mut rx_b = b.start().await.unwrap();
        let mut rx_c = c.start().await.unwrap();

        a.send_unicast(b"one", &[2]).await.unwrap();
        let received = rx_b.recv().await.unwrap();
        assert_eq!(received.npdu, Bytes::from_static(b"one"));
        assert_eq!(received.source_mac.as_ref(), &[1]);
        assert!(!received.link_layer_group);
        assert!(rx_a.try_recv().is_err());
        assert!(rx_c.try_recv().is_err());

        a.send_broadcast(b"all").await.unwrap();
        for receiver in [&mut rx_b, &mut rx_c] {
            let received = receiver.recv().await.unwrap();
            assert_eq!(received.npdu, Bytes::from_static(b"all"));
            assert_eq!(received.source_mac.as_ref(), &[1]);
            assert!(received.link_layer_group);
            assert!(receiver.try_recv().is_err());
        }
        assert!(rx_a.try_recv().is_err(), "broadcast must not echo");
        assert_eq!(
            a.stats(),
            VirtualNetworkStats {
                unicast_delivered: 1,
                broadcast_copies_delivered: 2,
                ..VirtualNetworkStats::default()
            }
        );
    }

    #[tokio::test]
    async fn unknown_destination_is_typed_and_observable() {
        let mut member = VirtualNetwork::join(name("unknown"), 1).unwrap();
        let _rx = member.start().await.unwrap();
        let error = member.send_unicast(b"lost", &[99]).await.unwrap_err();
        assert_eq!(io_kind(&error), Some(io::ErrorKind::NotFound));
        assert_eq!(member.stats().unknown_destination_drops, 1);
        assert!(matches!(
            member.send_unicast(b"bad-mac", &[1, 2]).await,
            Err(Error::OutOfRange(_))
        ));
    }

    #[test]
    fn duplicate_live_mac_is_atomic_but_names_isolate_networks() {
        let first_name = name("duplicate");
        let other_name = name("duplicate-other");
        let _first = VirtualNetwork::join(&first_name, 7).unwrap();
        let duplicate = match VirtualNetwork::join(&first_name, 7) {
            Err(error) => error,
            Ok(_) => panic!("duplicate MAC unexpectedly joined"),
        };
        assert_eq!(io_kind(&duplicate), Some(io::ErrorKind::AddrInUse));
        let other = VirtualNetwork::join(&other_name, 7).unwrap();
        assert_eq!(other.local_mac(), &[7]);
    }

    #[tokio::test]
    async fn stop_drop_and_rejoin_clean_up_membership() {
        let network = name("rejoin");
        let mut first = VirtualNetwork::join(&network, 8).unwrap();
        let _rx = first.start().await.unwrap();
        assert!(first.start().await.is_err());
        first.stop().await.unwrap();
        assert!(!lock_registry().networks.contains_key(&network));
        assert_eq!(
            io_kind(&first.send_broadcast(b"after-stop").await.unwrap_err()),
            Some(io::ErrorKind::NotConnected)
        );
        assert_eq!(
            io_kind(&first.start().await.unwrap_err()),
            Some(io::ErrorKind::NotConnected)
        );

        let replacement = VirtualNetwork::join(&network, 8).unwrap();
        drop(replacement);
        assert!(!lock_registry().networks.contains_key(&network));
        let replacement_after_drop = VirtualNetwork::join(&network, 8).unwrap();
        drop(replacement_after_drop);
    }

    #[test]
    fn abort_releases_membership_for_router_restart() {
        let network = name("abort-rejoin");
        let mut first = VirtualNetwork::join(&network, 8).unwrap();

        first.abort();
        let replacement = VirtualNetwork::join(&network, 8).unwrap();
        drop(replacement);
        assert!(!lock_registry().networks.contains_key(&network));
    }

    #[test]
    fn stale_generation_cannot_unregister_rejoined_member() {
        let network_name = name("aba");
        let first = VirtualNetwork::join(&network_name, 9).unwrap();
        let stale_generation = first.generation;
        drop(first);
        let replacement = VirtualNetwork::join(&network_name, 9).unwrap();

        unregister(&network_name, 9, stale_generation);
        let duplicate = match VirtualNetwork::join(&network_name, 9) {
            Err(error) => error,
            Ok(_) => panic!("stale generation removed the replacement"),
        };
        assert_eq!(io_kind(&duplicate), Some(io::ErrorKind::AddrInUse));
        drop(replacement);
    }

    #[tokio::test]
    async fn full_member_does_not_block_healthy_broadcast_recipient() {
        let network = name("full-isolation");
        let mut sender = VirtualNetwork::join(&network, 1).unwrap();
        let mut slow = VirtualNetwork::join_with_capacity(&network, 2, 1).unwrap();
        let mut healthy = VirtualNetwork::join(&network, 3).unwrap();
        let _sender_rx = sender.start().await.unwrap();
        let _slow_rx = slow.start().await.unwrap();
        let mut healthy_rx = healthy.start().await.unwrap();

        sender.send_unicast(b"fill", &[2]).await.unwrap();
        let error = sender.send_broadcast(b"broadcast").await.unwrap_err();
        assert_eq!(io_kind(&error), Some(io::ErrorKind::WouldBlock));
        let received = healthy_rx.recv().await.unwrap();
        assert_eq!(received.npdu, Bytes::from_static(b"broadcast"));
        assert!(received.link_layer_group);
        assert_eq!(sender.stats().full_member_drops, 1);
        assert_eq!(sender.stats().broadcast_copies_delivered, 1);
    }

    #[tokio::test]
    async fn pruned_generation_cannot_spoof_or_exclude_replacement() {
        let network = name("stale-sender");
        let mut stale_a = VirtualNetwork::join(&network, 1).unwrap();
        let mut b = VirtualNetwork::join(&network, 2).unwrap();
        let rx_a = stale_a.start().await.unwrap();
        let mut rx_b = b.start().await.unwrap();

        drop(rx_a);
        let cleanup_error = b.send_unicast(b"probe", &[1]).await.unwrap_err();
        assert_eq!(io_kind(&cleanup_error), Some(io::ErrorKind::NotConnected));

        let mut replacement_a = VirtualNetwork::join(&network, 1).unwrap();
        let mut replacement_rx = replacement_a.start().await.unwrap();
        let stale_unicast = stale_a.send_unicast(b"spoof", &[2]).await.unwrap_err();
        assert_eq!(io_kind(&stale_unicast), Some(io::ErrorKind::NotConnected));
        let stale_broadcast = stale_a.send_broadcast(b"spoof-all").await.unwrap_err();
        assert_eq!(io_kind(&stale_broadcast), Some(io::ErrorKind::NotConnected));
        assert!(rx_b.try_recv().is_err());
        assert!(replacement_rx.try_recv().is_err());

        drop(stale_a);
        b.send_broadcast(b"current").await.unwrap();
        let received = replacement_rx.recv().await.unwrap();
        assert_eq!(received.npdu, Bytes::from_static(b"current"));
        assert_eq!(received.source_mac.as_ref(), &[2]);
        assert!(received.link_layer_group);
        assert!(replacement_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn different_networks_do_not_deliver_to_each_other() {
        let mut a = VirtualNetwork::join(name("isolated-a"), 1).unwrap();
        let mut b = VirtualNetwork::join(name("isolated-b"), 2).unwrap();
        let _rx_a = a.start().await.unwrap();
        let mut rx_b = b.start().await.unwrap();
        let error = a.send_unicast(b"no-crossing", &[2]).await.unwrap_err();
        assert_eq!(io_kind(&error), Some(io::ErrorKind::NotFound));
        assert!(rx_b.try_recv().is_err());
    }

    #[test]
    fn validates_name_and_capacity_bounds() {
        assert!(matches!(
            VirtualNetwork::join("  ", 1),
            Err(Error::OutOfRange(_))
        ));
        assert!(matches!(
            VirtualNetwork::join_with_capacity(name("zero"), 1, 0),
            Err(Error::OutOfRange(_))
        ));
        assert!(matches!(
            VirtualNetwork::join_with_capacity(
                name("too-large"),
                1,
                MAX_VIRTUAL_NETWORK_QUEUE_CAPACITY + 1
            ),
            Err(Error::OutOfRange(_))
        ));
    }
}
