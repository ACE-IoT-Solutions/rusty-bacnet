//! What a node sends for an NPDU: the message and its destination, within
//! the datagram budget.

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_bip_codec::{Bvll, HEADER_LENGTH, MAX_BVLL_LENGTH};

use super::config::{Link, Mode, MAX_NPDU};

/// A message to send. The caller encodes it
/// ([`Bvll::encode_into`](bacnet_bip_codec::Bvll::encode_into)) and sends
/// it from the node's own socket to `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing<'n> {
    /// The destination B/IP address.
    pub to: SocketAddrV4,
    /// The message.
    pub message: Bvll<'n>,
}

/// How an NPDU is addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cast {
    /// To one node.
    Unicast,
    /// To every node on the network.
    Broadcast,
}

/// Why a send was refused. Nothing went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRefusal {
    /// The NPDU is longer than [`Link::max_npdu`] allows.
    TooLong {
        /// The longest NPDU this cast takes.
        max: usize,
    },
    /// A foreign device holds as many broadcasts as its caller can keep
    /// ([`Foreign::hold`](super::Foreign::hold)) until its first
    /// registration attempt is answered.
    HoldFull,
}

impl fmt::Display for SendRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { max } => write!(f, "NPDU longer than the {max} octets B/IP sends"),
            Self::HoldFull => f.write_str("no room to hold a broadcast until registration"),
        }
    }
}

impl core::error::Error for SendRefusal {}

/// Octets a broadcast's NPDU gains, at worst, before it reaches the
/// network's devices: the 4-octet header, and the 6-octet origin a BBMD
/// adds when it forwards it.
const BROADCAST_OVERHEAD: usize = HEADER_LENGTH + 6;

impl Link {
    /// The longest NPDU sent as `cast`.
    ///
    /// With a budget, a unicast's datagram is the NPDU plus the 4-octet
    /// header, and a broadcast's, once a BBMD forwards it as a
    /// Forwarded-NPDU, 6 octets more; either way at most [`MAX_NPDU`]
    /// (Table 6-1). Without one, what the Length field can state.
    #[must_use]
    pub fn max_npdu(&self, cast: Cast) -> usize {
        let overhead = match cast {
            Cast::Unicast => HEADER_LENGTH,
            Cast::Broadcast => BROADCAST_OVERHEAD,
        };
        match self.budget {
            Some(budget) => usize::from(budget).saturating_sub(overhead).min(MAX_NPDU),
            None => MAX_BVLL_LENGTH - overhead,
        }
    }

    /// An Original-Unicast-NPDU carrying `npdu` to `to` (Annex J.3).
    pub fn unicast<'n>(
        &self,
        to: SocketAddrV4,
        npdu: &'n [u8],
    ) -> Result<Outgoing<'n>, SendRefusal> {
        self.fits(npdu, Cast::Unicast)?;
        Ok(Outgoing {
            to,
            message: Bvll::OriginalUnicastNpdu(npdu),
        })
    }

    /// A local broadcast of `npdu`, by mode: an Original-Broadcast-NPDU to
    /// the subnet's broadcast address for a normal node or a BBMD (Annex
    /// J.4.1), and a Distribute-Broadcast-To-Network to its BBMD for a
    /// foreign device (Annex J.2.10). The flag is set for a BBMD, which
    /// also forwards its own broadcast
    /// ([`Tables::own_broadcast`](crate::bbmd::Tables::own_broadcast)).
    ///
    /// A foreign device asks [`Node::admit_broadcast`](super::Node::admit_broadcast)
    /// first.
    pub fn broadcast<'n>(&self, npdu: &'n [u8]) -> Result<(Outgoing<'n>, bool), SendRefusal> {
        self.fits(npdu, Cast::Broadcast)?;
        let local = Outgoing {
            to: self.subnet.broadcast,
            message: Bvll::OriginalBroadcastNpdu(npdu),
        };
        Ok(match self.mode {
            Mode::Normal => (local, false),
            Mode::Bbmd => (local, true),
            Mode::Foreign(foreign) => (
                Outgoing {
                    to: foreign.bbmd,
                    message: Bvll::DistributeBroadcastToNetwork(npdu),
                },
                false,
            ),
        })
    }

    fn fits(&self, npdu: &[u8], cast: Cast) -> Result<(), SendRefusal> {
        if self.budget.is_none() {
            return Ok(());
        }
        let max = self.max_npdu(cast);
        if npdu.len() > max {
            return Err(SendRefusal::TooLong { max });
        }
        Ok(())
    }
}
