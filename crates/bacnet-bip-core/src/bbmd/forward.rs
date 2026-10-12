//! Where a BBMD sends a broadcast it forwards (Annex J.4.5): pure decisions
//! over the two tables, written into the caller's storage.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BvlcResultCode, Bvll};

use core::fmt;

use super::bdt::{distribution_address, Bdt, TWO_HOP_MASK};
use super::fdt::Fdt;
use crate::time::Instant;
use crate::Arrival;

/// What kind of destination a [`Target`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Via {
    /// The B/IP broadcast address of this BBMD's own subnet.
    LocalBroadcast,
    /// A peer BBMD's subnet, through its BDT row: the subnet's directed
    /// broadcast address for one-hop distribution, the peer itself for
    /// two-hop.
    Peer,
    /// A registered foreign device.
    ForeignDevice,
}

/// One destination of a forwarded message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Where to send it.
    pub destination: SocketAddrV4,
    /// What kind of destination that is.
    pub via: Via,
}

impl Target {
    /// A placeholder to fill caller storage with before a call writes it.
    /// It is all zero bits, so a static filled with it can go in `.bss`.
    pub const UNUSED: Self = Self {
        destination: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
        via: Via::LocalBroadcast,
    };
}

/// What every forwarding decision needs besides the tables and the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// The caller's current time; registrations lapsed by then are skipped.
    pub now: Instant,
    /// Most foreign devices one message goes to; the rest are skipped and
    /// [`Fanout::fanout_capped`] is set. A local budget, not Annex J's.
    pub fdt_fanout: usize,
}

/// This BBMD's own IP subnet, for the decisions that can send to it:
/// [`Tables::forwarded_npdu`] and [`Tables::distribute_broadcast`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSubnet {
    /// The subnet's B/IP broadcast address: its directed broadcast address
    /// (or 255.255.255.255) at the B/IP port.
    pub broadcast: SocketAddrV4,
    /// The mask of the subnet this BBMD's own B/IP address is on, when the
    /// caller knows it. With it, a peer's Forwarded-NPDU sent from an
    /// address on the subnet isn't broadcast there again: the subnet heard
    /// it first hand. Leave it `None` when it isn't known, or when the own
    /// address isn't on the subnet, as behind NAT.
    pub mask: Option<Ipv4Addr>,
}

/// Destinations written to the caller's storage, and how the list was cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fanout<'o> {
    /// The destinations, in order: the local broadcast if any, then BDT
    /// peers in table order, then foreign devices in table order.
    pub targets: &'o [Target],
    /// Destinations left out because an earlier one had the same address,
    /// as when two BDT rows share a directed broadcast address.
    pub deduplicated: usize,
    /// Whether [`Context::fdt_fanout`] cut the foreign devices short.
    pub fanout_capped: bool,
    /// Destinations lost because the storage was full. Storage of
    /// [`Tables::MAX_TARGETS`] never loses any.
    pub dropped: usize,
}

/// A Forwarded-NPDU to send, and where (Annex J.2.5, J.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distribution<'n, 'o> {
    /// The message's originating B/IP address.
    pub origin: SocketAddrV4,
    /// The NPDU it carries.
    pub npdu: &'n [u8],
    /// Its destinations.
    pub fanout: Fanout<'o>,
}

impl<'n, 'o> Distribution<'n, 'o> {
    /// The Forwarded-NPDU every destination gets.
    #[must_use]
    pub const fn message(&self) -> Bvll<'n> {
        Bvll::ForwardedNpdu {
            origin: self.origin,
            npdu: self.npdu,
        }
    }

    /// Each destination with the message to send it.
    pub fn transmissions(
        &self,
    ) -> impl ExactSizeIterator<Item = (SocketAddrV4, Bvll<'n>)> + use<'n, 'o> {
        let message = self.message();
        self.fanout
            .targets
            .iter()
            .map(move |target| (target.destination, message))
    }
}

/// Why a BBMD neither delivers nor forwards a broadcast it received (Annex
/// J.4.5). [`ForwardRefusal::result_code`] gives the BVLC-Result to answer
/// the sender with, where Annex J has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardRefusal {
    /// A Forwarded-NPDU from a sender with no BDT row: only peers' are
    /// acted on. It goes unanswered.
    NotFromPeer,
    /// A Forwarded-NPDU from this BBMD's own address: the echo of its own
    /// rebroadcast, which its own row would otherwise pass as a peer's. It
    /// goes unanswered.
    OwnEcho,
    /// A Distribute-Broadcast-To-Network from a sender holding no live
    /// registration. It is answered with a NAK.
    NotRegistered,
}

impl ForwardRefusal {
    /// The BVLC-Result to answer the sender with, or `None` when the
    /// message goes unanswered.
    #[must_use]
    pub const fn result_code(self) -> Option<BvlcResultCode> {
        match self {
            Self::NotFromPeer | Self::OwnEcho => None,
            Self::NotRegistered => Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK),
        }
    }
}

impl fmt::Display for ForwardRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFromPeer => "Forwarded-NPDU from a sender that is no BDT peer",
            Self::OwnEcho => "Forwarded-NPDU from this BBMD's own address",
            Self::NotRegistered => {
                "Distribute-Broadcast-To-Network from a sender with no live registration"
            }
        })
    }
}

impl core::error::Error for ForwardRefusal {}

/// A BBMD's two tables, and its forwarding decisions over them.
///
/// The fields are public so a caller can update one table while reading the
/// other.
#[derive(Debug, Clone)]
pub struct Tables<const B: usize, const F: usize> {
    /// The broadcast distribution table, which holds this BBMD's own address.
    pub bdt: Bdt<B>,
    /// The foreign device table.
    pub fdt: Fdt<F>,
}

impl<const B: usize, const F: usize> Tables<B, F> {
    /// The most destinations one decision writes: every BDT row and foreign
    /// device, and the local broadcast. Storage this long never drops one.
    pub const MAX_TARGETS: usize = B + F + 1;

    /// Empty tables for the BBMD at `own` (see [`Bdt::new`]).
    #[must_use]
    pub const fn new(own: SocketAddrV4) -> Self {
        Self {
            bdt: Bdt::new(own),
            fdt: Fdt::new(),
        }
    }

    /// Where a broadcast that starts on this BBMD's subnet goes, other than
    /// that subnet: each BDT row's [`distribution_address`] and each live
    /// foreign device, up to [`Context::fdt_fanout`] of those, skipping this
    /// BBMD's own row and `exclude` (the broadcast's source) in both tables.
    /// An address already listed is left out.
    pub fn broadcast_targets<'o>(
        &self,
        cx: &Context,
        exclude: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Fanout<'o> {
        let mut writer = Writer::new(out);
        self.peer_targets(&mut writer, exclude);
        self.foreign_device_targets_into(&mut writer, cx, exclude);
        writer.finish()
    }

    /// The live foreign devices but `exclude`, up to [`Context::fdt_fanout`]
    /// of them.
    pub fn foreign_device_targets<'o>(
        &self,
        cx: &Context,
        exclude: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Fanout<'o> {
        let mut writer = Writer::new(out);
        self.foreign_device_targets_into(&mut writer, cx, exclude);
        writer.finish()
    }

    /// An Original-Broadcast-NPDU from `source` on this subnet (Annex
    /// J.4.5): a Forwarded-NPDU with `source` as its origin to the
    /// [`broadcast_targets`](Self::broadcast_targets) excluding `source`.
    /// The subnet already has it, so the local broadcast address isn't one.
    pub fn original_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        source: SocketAddrV4,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Distribution<'n, 'o> {
        Distribution {
            origin: source,
            npdu,
            fanout: self.broadcast_targets(cx, source, out),
        }
    }

    /// A broadcast this BBMD's own device sends on its subnet: what
    /// [`original_broadcast`](Self::original_broadcast) does for one from
    /// this BBMD's own address.
    pub fn own_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Distribution<'n, 'o> {
        self.original_broadcast(cx, self.bdt.own_address(), npdu, out)
    }

    /// A Forwarded-NPDU that `sender` sent this BBMD, whose originating
    /// address is `origin` (Annex J.4.5).
    ///
    /// Only a BDT peer's is accepted; the caller delivers an accepted one
    /// to its own network layer and sends it on unchanged, origin and all,
    /// to the destinations returned: the live foreign devices but `origin`
    /// and, when [`rebroadcasts_forwarded`](Self::rebroadcasts_forwarded)
    /// says so, the subnet's broadcast address first. The peers don't get
    /// it again.
    pub fn forwarded_npdu<'o>(
        &self,
        cx: &Context,
        subnet: &LocalSubnet,
        sender: SocketAddrV4,
        arrival: Arrival,
        origin: SocketAddrV4,
        out: &'o mut [Target],
    ) -> Result<Fanout<'o>, ForwardRefusal> {
        if sender == self.bdt.own_address() {
            return Err(ForwardRefusal::OwnEcho);
        }
        if !self.bdt.contains(sender) {
            return Err(ForwardRefusal::NotFromPeer);
        }
        let mut writer = Writer::new(out);
        if self.rebroadcasts_forwarded(subnet.mask, sender, arrival) {
            writer.push_local(subnet.broadcast);
        }
        self.foreign_device_targets_into(&mut writer, cx, origin);
        Ok(writer.finish())
    }

    /// Whether a peer's Forwarded-NPDU from `sender` still has to be
    /// broadcast on this BBMD's subnet (Annex J.4.5): only when it came to
    /// this BBMD alone.
    ///
    /// Peers address this subnet through their copy of this BBMD's own BDT
    /// row, which every BDT gives the same mask (J.4.3.2). With
    /// [`TWO_HOP_MASK`] they send to this BBMD by unicast; with the subnet's
    /// mask, to the subnet's directed broadcast address, so every device
    /// there has it already. A datagram that `arrival`ed by broadcast, or
    /// whose sender is on this BBMD's subnet (by `subnet_mask`, when known),
    /// reached the subnet too. This doesn't check that `sender` is a peer;
    /// [`forwarded_npdu`](Self::forwarded_npdu) does.
    #[must_use]
    pub fn rebroadcasts_forwarded(
        &self,
        subnet_mask: Option<Ipv4Addr>,
        sender: SocketAddrV4,
        arrival: Arrival,
    ) -> bool {
        let own = self.bdt.own_address();
        let peers_send_here = self.bdt.row(own).is_none_or(|row| row.mask == TWO_HOP_MASK);
        let sender_on_subnet = subnet_mask
            .is_some_and(|mask| (sender.ip().to_bits() ^ own.ip().to_bits()) & mask.to_bits() == 0);
        peers_send_here && arrival == Arrival::Unicast && !sender_on_subnet
    }

    /// A Distribute-Broadcast-To-Network from `sender` (Annex J.4.5).
    ///
    /// Only a foreign device registered and live at [`Context::now`] may ask;
    /// anyone else is refused with [`ForwardRefusal::NotRegistered`], whose
    /// [`result_code`](ForwardRefusal::result_code) is the NAK to send. The caller delivers
    /// an accepted one to its own network layer. It goes out as a
    /// Forwarded-NPDU with `sender` as its origin: first to the subnet's
    /// broadcast address, then to the
    /// [`broadcast_targets`](Self::broadcast_targets) excluding `sender`.
    pub fn distribute_broadcast<'n, 'o>(
        &self,
        cx: &Context,
        subnet: &LocalSubnet,
        sender: SocketAddrV4,
        npdu: &'n [u8],
        out: &'o mut [Target],
    ) -> Result<Distribution<'n, 'o>, ForwardRefusal> {
        if !self.fdt.is_registered_at(sender, cx.now) {
            return Err(ForwardRefusal::NotRegistered);
        }
        let mut writer = Writer::new(out);
        writer.push_local(subnet.broadcast);
        self.peer_targets(&mut writer, sender);
        self.foreign_device_targets_into(&mut writer, cx, sender);
        Ok(Distribution {
            origin: sender,
            npdu,
            fanout: writer.finish(),
        })
    }

    /// The BDT rows but this BBMD's own and `exclude`.
    fn peer_targets(&self, writer: &mut Writer<'_>, exclude: SocketAddrV4) {
        let own = self.bdt.own_address();
        for row in self.bdt.rows() {
            if row.address == own || row.address == exclude {
                continue;
            }
            writer.push_unique(distribution_address(*row), Via::Peer);
        }
        writer.peers_end = writer.len;
    }

    /// The live foreign devices but `exclude`, after any peers.
    fn foreign_device_targets_into(
        &self,
        writer: &mut Writer<'_>,
        cx: &Context,
        exclude: SocketAddrV4,
    ) {
        let mut sent = 0;
        for entry in self.fdt.live_at(cx.now) {
            if entry.address == exclude {
                continue;
            }
            // The table holds one registration per address, so only a peer
            // target can repeat one.
            if writer.is_peer_target(entry.address) {
                writer.deduplicated += 1;
                continue;
            }
            if sent >= cx.fdt_fanout {
                writer.fanout_capped = true;
                break;
            }
            writer.push(entry.address, Via::ForeignDevice);
            sent += 1;
        }
    }
}

/// Fills the caller's storage for one decision.
struct Writer<'o> {
    out: &'o mut [Target],
    len: usize,
    /// Where the peer targets start: after the local broadcast, which is
    /// never compared with them.
    peers_start: usize,
    /// Where they end.
    peers_end: usize,
    deduplicated: usize,
    fanout_capped: bool,
    dropped: usize,
}

impl<'o> Writer<'o> {
    fn new(out: &'o mut [Target]) -> Self {
        Self {
            out,
            len: 0,
            peers_start: 0,
            peers_end: 0,
            deduplicated: 0,
            fanout_capped: false,
            dropped: 0,
        }
    }

    fn push(&mut self, destination: SocketAddrV4, via: Via) {
        match self.out.get_mut(self.len) {
            Some(slot) => {
                *slot = Target { destination, via };
                self.len += 1;
            }
            None => self.dropped += 1,
        }
    }

    /// The local broadcast, first.
    fn push_local(&mut self, destination: SocketAddrV4) {
        self.push(destination, Via::LocalBroadcast);
        self.peers_start = self.len;
        self.peers_end = self.len;
    }

    /// A peer target, unless an earlier one has its address.
    fn push_unique(&mut self, destination: SocketAddrV4, via: Via) {
        if self.out[self.peers_start..self.len]
            .iter()
            .any(|target| target.destination == destination)
        {
            self.deduplicated += 1;
        } else {
            self.push(destination, via);
        }
    }

    fn is_peer_target(&self, destination: SocketAddrV4) -> bool {
        self.out[self.peers_start..self.peers_end]
            .iter()
            .any(|target| target.destination == destination)
    }

    fn finish(self) -> Fanout<'o> {
        let Self {
            out,
            len,
            deduplicated,
            fanout_capped,
            dropped,
            ..
        } = self;
        Fanout {
            targets: &out[..len],
            deduplicated,
            fanout_capped,
            dropped,
        }
    }
}
