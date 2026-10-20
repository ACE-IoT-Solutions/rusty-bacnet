//! B/IP multicast for a node (Annex J.8, #941): a B/IP-M node sends its
//! broadcasts to an IPv4 multicast group, which stands in for its subnet's
//! broadcast address, and takes broadcasts only from that group.
//!
//! **Configuration.** [`BipMulticast`] names the group (class D only), the
//! multicast TTL (1 by default, so the group stays on the local network:
//! raise it for a group that spans IP routers, which must forward
//! multicast) and the group's UDP port (the transport's own by default).
//! A node in multicast mode needs an explicit interface: the group is
//! joined there and multicasts leave from it. There is no wildcard bind and
//! no route lookup to pick one.
//!
//! **Sockets.** The primary socket sends to the group from the interface
//! (`IP_MULTICAST_IF`), with the TTL, and with loop on, so several stacks
//! on one host can share a group: each drops its own looped-back copy, as
//! it drops any datagram from its own address. It never takes the group's
//! traffic itself: on Linux it turns `IP_MULTICAST_ALL` off, so it gets no
//! group another socket joined. On Linux, macOS and the BSDs a receive-only
//! listener binds the group address and port with `SO_REUSEADDR` (and
//! `SO_REUSEPORT` on macOS and the BSDs) and joins the group on the
//! interface. Each stack on the host has its own listener and the kernel
//! hands each a copy; the receive loop reads it beside the primary socket,
//! in arrival order (#1546). Windows can't bind a group address, so there
//! the primary socket joins the group itself and the group's port must be
//! the transport's own; it reports the group delivery in the receive
//! flags. Windows multicast is untested.
//!
//! The group is left when its socket closes: when the transport stops, or
//! when an activation replaces or ends the group.
//!
//! **Network Port.** A port that can use multicast serves
//! BACnet_IP_Multicast_Address (Clause 12.56.26): the group, or 0.0.0.0
//! while off. A written group waits for a ReinitializeDevice, which joins
//! it at run time through [`MulticastControl`]: a new listener is bound and
//! joined before anything changes, then the link moves to the group and the
//! receive loop takes the listener in place of the old one. A later start
//! uses the group activated last.
//!
//! Lock order: the object database (held by the caller), then the
//! control's lock, then the node's. No lock here is held across an await.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use bacnet_bip_core::LocalSubnet;
use bacnet_types::bip_port::IpMulticast;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::Notify;
use tracing::{info, warn};

use super::node::NodeHandle;

/// A B/IP-M node's group (Annex J.8): where its broadcasts go and come
/// from. See [`BipTransport::set_multicast`](super::BipTransport::set_multicast).
///
/// ```
/// use std::net::Ipv4Addr;
/// use bacnet_transport::bip::BipMulticast;
///
/// // A group whose members sit on two IP subnets one router apart.
/// let group = BipMulticast::new(Ipv4Addr::new(239, 255, 66, 1))?.with_ttl(2)?;
/// assert_eq!((group.ttl(), group.port()), (2, None));
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BipMulticast {
    group: Ipv4Addr,
    ttl: u8,
    port: Option<u16>,
}

impl BipMulticast {
    /// The default multicast TTL: the group's datagrams stay on the local
    /// network.
    pub const DEFAULT_TTL: u8 = 1;

    /// The group `group` at the transport's own UDP port, with
    /// [`DEFAULT_TTL`](Self::DEFAULT_TTL). Fails unless `group` is a class D
    /// (multicast) address.
    pub fn new(group: Ipv4Addr) -> Result<Self, Error> {
        if !group.is_multicast() {
            return Err(config(format!(
                "a B/IP multicast group must be a class D address (224.0.0.0/4), not {group}"
            )));
        }
        Ok(Self {
            group,
            ttl: Self::DEFAULT_TTL,
            port: None,
        })
    }

    /// This group with multicast TTL `ttl`, 1 to 255. Raise it above 1 for
    /// a group whose members are more IP routers apart than that.
    pub fn with_ttl(self, ttl: u8) -> Result<Self, Error> {
        if ttl == 0 {
            return Err(config(
                "a B/IP multicast TTL must be 1 to 255; 0 would keep it on this host".into(),
            ));
        }
        Ok(Self { ttl, ..self })
    }

    /// This group at UDP port `port` rather than the transport's own: the
    /// port every member sends its broadcasts to. Members on one host, at
    /// one IP address, need their own unicast ports and so this one. Not
    /// on Windows, where the group's port is the transport's.
    pub fn with_port(self, port: u16) -> Result<Self, Error> {
        if port == 0 {
            return Err(config(
                "a B/IP multicast group's port must be nonzero".into(),
            ));
        }
        Ok(Self {
            port: Some(port),
            ..self
        })
    }

    /// The group address.
    pub fn group(&self) -> Ipv4Addr {
        self.group
    }

    /// The multicast TTL.
    pub fn ttl(&self) -> u8 {
        self.ttl
    }

    /// The group's UDP port, or `None` for the transport's own.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The group's B/IP address for a transport bound to `own_port`.
    pub(super) fn endpoint(&self, own_port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(self.group, self.port.unwrap_or(own_port))
    }
}

impl fmt::Display for BipMulticast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(port) => write!(f, "{}:{port} (TTL {})", self.group, self.ttl),
            None => write!(f, "{} (TTL {})", self.group, self.ttl),
        }
    }
}

fn config(message: String) -> Error {
    Error::Encoding(message)
}

/// Why a node bound to `interface` at `transport_port` (0 for a port the
/// OS picks) can't use `multicast`: checked when it is configured, at
/// start and for a group written to the Network Port.
fn refusal(interface: Ipv4Addr, transport_port: u16, multicast: &BipMulticast) -> Option<String> {
    if interface.is_unspecified() {
        return Some(
            "B/IP multicast needs an explicit interface address to join the group on; \
             0.0.0.0 is refused"
                .into(),
        );
    }
    if cfg!(windows) && multicast.port.is_some_and(|port| port != transport_port) {
        return Some("on Windows a B/IP multicast group's port must be the transport's own".into());
    }
    if multicast.port.is_none() && transport_port == 0 {
        return Some(
            "a B/IP multicast group needs a port: give the transport a nonzero port or the \
             group its own (BipMulticast::with_port)"
                .into(),
        );
    }
    None
}

/// A new group listener for the receive loop, or none: the hand-off from an
/// activation to the loop that reads it (unused on Windows, whose primary
/// socket joins the group).
#[derive(Default)]
pub(super) struct GroupSwap {
    next: Mutex<Option<Option<socket2::Socket>>>,
    ready: Notify,
}

impl GroupSwap {
    /// Hand the loop `listener`, replacing the one it reads; `None` closes
    /// it.
    fn offer(&self, listener: Option<socket2::Socket>) {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) = Some(listener);
        self.ready.notify_one();
    }

    /// The next listener offered. Cancel-safe: the offer is taken in the
    /// same poll that sees it.
    pub(super) async fn next(&self) -> Option<socket2::Socket> {
        loop {
            self.ready.notified().await;
            if let Some(listener) = self
                .next
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                return listener;
            }
        }
    }
}

/// The primary socket's multicast options: send from `interface` with
/// `ttl`, loop on, and on Linux no group it didn't join.
pub(super) fn configure_sender(
    socket: socket2::SockRef<'_>,
    interface: Ipv4Addr,
    ttl: u8,
) -> io::Result<()> {
    socket.set_multicast_if_v4(&interface)?;
    socket.set_multicast_ttl_v4(u32::from(ttl))?;
    socket.set_multicast_loop_v4(true)?;
    #[cfg(target_os = "linux")]
    socket.set_multicast_all_v4(false)?;
    Ok(())
}

/// A receive-only socket on `group`'s address and port, joined to it on
/// `interface`.
#[cfg(not(windows))]
pub(super) fn group_listener(
    group: SocketAddrV4,
    interface: Ipv4Addr,
) -> io::Result<socket2::Socket> {
    let socket = super::socket::udp_socket(super::socket::SocketRole::GroupListener)?;
    socket.bind(&group.into()).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("could not bind the B/IP multicast listener to {group} ({e})"),
        )
    })?;
    socket
        .join_multicast_v4(group.ip(), &interface)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "could not join B/IP multicast group {} on {interface} ({e})",
                    group.ip()
                ),
            )
        })?;
    #[cfg(target_os = "linux")]
    socket.set_multicast_all_v4(false)?;
    Ok(socket)
}

/// What the control's lock guards.
struct ControlState {
    /// The group in effect, and the one the next start uses.
    config: Option<BipMulticast>,
    /// The TTL and port a group activated through the Network Port gets:
    /// the last configured.
    ttl: u8,
    port: Option<u16>,
    /// The running transport, while one runs.
    running: Option<Running>,
}

/// What an activation changes in a running transport.
struct Running {
    node: Weak<NodeHandle>,
    /// The bound UDP port.
    port: u16,
    /// Where broadcasts go without a group: the configured broadcast
    /// address at the bound port.
    subnet: SocketAddrV4,
    /// The receive loop's listener hand-off.
    swap: Arc<GroupSwap>,
}

/// One transport's multicast group, shared with the Network Port object it
/// is lent to: the group in effect and, while the transport runs, how to
/// move it to another.
pub(crate) struct MulticastControl {
    state: Mutex<ControlState>,
    /// The transport's configured interface: the group is joined there.
    interface: Ipv4Addr,
    /// The transport's configured port, 0 when the OS picks it: a group at
    /// a picked port couldn't share it with the other members.
    configured_port: u16,
}

impl MulticastControl {
    pub(super) fn new(interface: Ipv4Addr, configured_port: u16) -> Self {
        Self {
            state: Mutex::new(ControlState {
                config: None,
                ttl: BipMulticast::DEFAULT_TTL,
                port: None,
                running: None,
            }),
            interface,
            configured_port,
        }
    }

    /// Why this transport can't use `multicast`, if it can't.
    pub(super) fn refusal(&self, multicast: &BipMulticast) -> Option<String> {
        refusal(self.interface, self.configured_port, multicast)
    }

    fn lock(&self) -> MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The group in effect.
    pub(super) fn config(&self) -> Option<BipMulticast> {
        self.lock().config
    }

    /// Configure the group, or none, for the next start.
    pub(super) fn set(&self, multicast: Option<BipMulticast>) {
        let mut state = self.lock();
        if let Some(multicast) = multicast {
            state.ttl = multicast.ttl;
            state.port = multicast.port;
        }
        state.config = multicast;
    }

    /// Hand the control the running transport, for activations.
    pub(super) fn attach(
        &self,
        node: &Arc<NodeHandle>,
        port: u16,
        subnet: SocketAddrV4,
        swap: Arc<GroupSwap>,
    ) {
        self.lock().running = Some(Running {
            node: Arc::downgrade(node),
            port,
            subnet,
            swap,
        });
    }

    /// Forget the running transport: it stopped.
    pub(super) fn detach(&self) {
        self.lock().running = None;
    }

    /// The running node: attached, and not stopped.
    fn running(state: &ControlState) -> Option<(Arc<NodeHandle>, &Running)> {
        let running = state.running.as_ref()?;
        let node = running.node.upgrade().filter(|node| !node.is_stopped())?;
        Some((node, running))
    }

    /// The group a written value names, as it would be activated: `None`
    /// for 0.0.0.0. Refused with the WriteProperty error a client gets.
    fn written(&self, state: &ControlState, group: [u8; 4]) -> Result<Option<BipMulticast>, Error> {
        let group = Ipv4Addr::from(group);
        if group.is_unspecified() {
            return Ok(None);
        }
        let mut multicast =
            BipMulticast::new(group).map_err(|_| protocol(ErrorCode::VALUE_OUT_OF_RANGE))?;
        multicast.ttl = state.ttl;
        multicast.port = state.port;
        if self.refusal(&multicast).is_some() {
            return Err(protocol(ErrorCode::VALUE_OUT_OF_RANGE));
        }
        Ok(Some(multicast))
    }
}

/// Ready the sockets for `next`: on Linux, macOS and the BSDs the sender's
/// options and a listener joined to the group, for the receive loop to take
/// in place of the old one (the old group is left when its listener
/// closes).
#[cfg(not(windows))]
fn move_sockets(
    node: &NodeHandle,
    interface: Ipv4Addr,
    _old: Option<BipMulticast>,
    next: Option<BipMulticast>,
    port: u16,
) -> io::Result<Option<socket2::Socket>> {
    let Some(next) = next else {
        return Ok(None);
    };
    configure_sender(
        socket2::SockRef::from(&***node.socket()),
        interface,
        next.ttl,
    )?;
    group_listener(next.endpoint(port), interface).map(Some)
}

/// Ready the sockets for `next`: on Windows the primary socket joins the
/// new group and leaves the old one, so there is no listener to hand over.
#[cfg(windows)]
fn move_sockets(
    node: &NodeHandle,
    interface: Ipv4Addr,
    old: Option<BipMulticast>,
    next: Option<BipMulticast>,
    _port: u16,
) -> io::Result<Option<socket2::Socket>> {
    let primary = socket2::SockRef::from(&***node.socket());
    if let Some(next) = next {
        configure_sender(
            socket2::SockRef::from(&***node.socket()),
            interface,
            next.ttl,
        )?;
        primary.join_multicast_v4(&next.group, &interface)?;
    }
    if let Some(old) = old {
        if let Err(error) = primary.leave_multicast_v4(&old.group, &interface) {
            warn!(%error, group = %old.group, "Could not leave the old B/IP multicast group");
        }
    }
    Ok(None)
}

fn protocol(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code,
    }
}

impl IpMulticast for MulticastControl {
    fn group(&self) -> [u8; 4] {
        self.lock()
            .config
            .map_or([0; 4], |multicast| multicast.group.octets())
    }

    fn check(&self, group: [u8; 4]) -> Result<(), Error> {
        let state = self.lock();
        if Self::running(&state).is_none() {
            return Err(protocol(ErrorCode::WRITE_ACCESS_DENIED));
        }
        self.written(&state, group).map(drop)
    }

    fn activate(&self, group: [u8; 4]) -> Result<(), Error> {
        let mut state = self.lock();
        let next = self.written(&state, group)?;
        if next == state.config {
            return Ok(());
        }
        let Some((node, running)) = Self::running(&state) else {
            return Err(protocol(ErrorCode::WRITE_ACCESS_DENIED));
        };
        let (port, swap) = (running.port, Arc::clone(&running.swap));
        let subnet = next.map_or(running.subnet, |multicast| multicast.endpoint(port));
        // The sockets first, since they can fail; then the link, which
        // can't once the group is valid.
        let moved =
            move_sockets(&node, self.interface, state.config, next, port).map_err(|error| {
                warn!(%error, "B/IP multicast group change failed; nothing changed");
                Error::Transport(error)
            })?;
        node.set_subnet(LocalSubnet::new(subnet))?;
        swap.offer(moved);
        info!(group = ?next.map(|multicast| multicast.group), "B/IP multicast group changed");
        state.config = next;
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}

impl super::BipTransport {
    /// Make this node a B/IP-M member of `multicast`'s group (Annex J.8), or,
    /// with `None`, not. Its broadcasts then go to the group as
    /// Original-Broadcast-NPDU, and it takes Original-Broadcast-NPDU and
    /// Forwarded-NPDU only from the group: never a subnet broadcast, since
    /// members and plain B/IP devices don't share a subnet (Annex J.8.1).
    /// Unicast is unchanged. Several stacks on one host can share a group:
    /// each needs its own unicast address or port, and each drops its own
    /// looped-back copies.
    ///
    /// Fails, changing nothing, on a transport bound to 0.0.0.0: the group
    /// is joined on the interface, which must be explicit. With no group
    /// port ([`BipMulticast::with_port`]) the transport's port must be
    /// nonzero. `start()` also refuses a member that is a BBMD (a BBMD that
    /// serves a group comes later, #941), registers as a foreign device, or
    /// shares its port by address. Call it before `start()`. A registered
    /// Network Port serves the group as BACnet_IP_Multicast_Address, which a
    /// ReinitializeDevice can change while the transport runs; a later start
    /// keeps that change.
    pub fn set_multicast(&mut self, multicast: Option<BipMulticast>) -> Result<(), Error> {
        if let Some(refused) = multicast.and_then(|group| self.multicast.refusal(&group)) {
            return Err(config(refused));
        }
        self.multicast.set(multicast);
        Ok(())
    }

    /// The B/IP-M group in effect, if any: the one configured, or the one a
    /// Network Port activation moved to.
    pub fn multicast(&self) -> Option<BipMulticast> {
        self.multicast.config()
    }

    /// Whether this transport can be a B/IP-M member, so it lends its group
    /// to the Network Port: an explicit interface, NORMAL mode, not sharing
    /// its port by address, and a port for the group.
    pub(super) fn supports_multicast(&self) -> bool {
        let group_port = self.multicast.config().and_then(|group| group.port);
        !self.interface.is_unspecified()
            && !self.share_port_by_address
            && self.foreign_device.is_none()
            && self.bbmd_config.is_none()
            && self.bbmd.is_none()
            && (self.multicast.configured_port != 0 || group_port.is_some())
    }

    /// Where this link's broadcasts go: the multicast group at its port, or
    /// the configured broadcast address at the link's port.
    pub(super) fn broadcast_endpoint(&self) -> SocketAddrV4 {
        match self.multicast.config() {
            Some(multicast) => multicast.endpoint(self.port),
            None => SocketAddrV4::new(self.broadcast_address, self.port),
        }
    }

    /// Refuse, before any I/O, a start of a member that can't be one.
    pub(super) fn check_multicast_start(
        &self,
        multicast: &BipMulticast,
        foreign: bool,
    ) -> Result<(), Error> {
        let refused = if self.bbmd_config.is_some() || self.bbmd.is_some() {
            Some(
                "a B/IP BBMD can't serve a multicast group yet (#941); run the member and the \
                 BBMD on separate transports"
                    .to_string(),
            )
        } else if foreign {
            Some(
                "a B/IP foreign device broadcasts through its BBMD, so it can't also be a \
                 multicast group member"
                    .to_string(),
            )
        } else if self.share_port_by_address {
            Some(
                "a B/IP multicast node can't also share its port by address; give each node \
                 on the host its own port and the group a shared one"
                    .to_string(),
            )
        } else {
            self.multicast.refusal(multicast)
        };
        refused.map_or(Ok(()), |refused| Err(config(refused)))
    }
}
