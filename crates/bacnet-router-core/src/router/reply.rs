//! Owned addressing for a response to one locally delivered request.
use crate::npdu::{Address, Npdu, WriteError};
use crate::{Mac, PortId};

/// A bounded response description independent of the engine and its scratch.
///
/// The return port and immediate peer are those of the request. Its SNET/SADR
/// becomes the response destination; an explicitly addressed router endpoint
/// becomes the source. No routing-table lookup or default source selection is
/// performed. This describes bytes and a port, not permission to use a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyPlan {
    port: PortId,
    peer: Mac,
    source: Option<(u16, Mac)>,
    destination: Option<(u16, Mac)>,
}

impl ReplyPlan {
    /// Validate and own one response's explicit addresses. Source network 0 or
    /// 65535, an empty source, and oversized addresses are refused. A response
    /// destination must also name one node on a non-reserved network.
    pub fn new(
        port: PortId,
        peer: &[u8],
        source: Option<Address<'_>>,
        destination: Option<Address<'_>>,
    ) -> Result<Self, WriteError> {
        if destination.is_some_and(|a| a.network == 65535 || a.mac.is_empty()) {
            return Err(if destination.is_some_and(|a| a.network == 65535) {
                WriteError::ReservedNetwork
            } else {
                WriteError::EmptyDestination
            });
        }
        // The existing writer is the field-validity oracle; no payload or
        // engine scratch is involved in this small header-only preflight.
        let mut header = [0; 2 + 2 * (3 + crate::MAX_MAC_LEN) + 1];
        Npdu {
            source,
            destination,
            ..Npdu::apdu(&[])
        }
        .write(&mut header)?;
        let own = |a: Address<'_>| -> Result<_, WriteError> {
            Ok((
                a.network,
                Mac::from_slice(a.mac).map_err(|_| WriteError::AddressTooLong)?,
            ))
        };
        Ok(Self {
            port,
            peer: Mac::from_slice(peer).map_err(|_| WriteError::AddressTooLong)?,
            source: source.map(own).transpose()?,
            destination: destination.map(own).transpose()?,
        })
    }

    /// Captured return port; it never follows a later route-table update.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }
    /// Captured immediate peer, distinct from the routed destination.
    #[must_use]
    pub fn peer(&self) -> &[u8] {
        self.peer.as_slice()
    }
    /// Explicit router endpoint addressed by the request, if any.
    #[must_use]
    pub fn source(&self) -> Option<Address<'_>> {
        self.source.as_ref().map(|(network, mac)| Address {
            network: *network,
            mac: mac.as_slice(),
        })
    }
    /// Original request SNET/SADR, if any.
    #[must_use]
    pub fn destination(&self) -> Option<Address<'_>> {
        self.destination.as_ref().map(|(network, mac)| Address {
            network: *network,
            mac: mac.as_slice(),
        })
    }
    /// Prepare the same NPDU view for sizing and writing into caller storage.
    /// The view borrows this owned plan and the supplied APDU, never the router.
    #[must_use]
    pub fn npdu<'a>(&'a self, apdu: &'a [u8], expecting_reply: bool, priority: u8) -> Npdu<'a> {
        Npdu {
            source: self.source(),
            destination: self.destination(),
            expecting_reply,
            priority,
            ..Npdu::apdu(apdu)
        }
    }
}
