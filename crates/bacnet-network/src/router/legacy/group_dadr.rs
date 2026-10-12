//! What the router does with an NPDU it would deliver to a DADR that is a
//! group on its delivery port (#1504, #1677).
//!
//! Only an Unconfirmed-Request may reach a group, and only one of the
//! groups the port's local broadcast stands for
//! (`TransportPort::local_broadcast_destinations`): it goes out as that
//! local broadcast, which is what its sender meant. B/IP refuses a unicast
//! to a group address, since its receivers drop one, and a group at another
//! UDP port belongs to another B/IP network. Anything else is dropped and
//! counted, with no reject: as one unicast an APDU would reach every node in
//! the group without the broadcast forms #1491 filters, and a network
//! message must not reach every router there as a broadcast its sender
//! never asked for. A DLEN 0 broadcast is not affected.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bacnet_encoding::npdu::Npdu;
use bacnet_transport::port::GroupDestinations;
use bacnet_types::enums::PduType;
use tracing::warn;

use super::router_table::RouteEntry;
use crate::layer::count_drop;

/// What becomes of an NPDU for a DADR that is a group on its delivery port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GroupDadr {
    /// The DADR isn't a group there: deliver it as addressed.
    No,
    /// An Unconfirmed-Request for one of the groups the port's local
    /// broadcast stands for: send it as that broadcast.
    Broadcast,
    /// Anything else: dropped and counted.
    Refused,
}

/// Each port's group destinations, by port index, the part of them its
/// local broadcast stands for, and where a drop is counted.
#[derive(Debug, Default)]
pub(super) struct GroupRules {
    groups: Vec<GroupDestinations>,
    local_broadcasts: Vec<GroupDestinations>,
    drops: Arc<AtomicU64>,
}

impl GroupRules {
    /// The rules of the ports, by port index, counting drops in `drops`.
    pub(super) fn new(
        groups: Vec<GroupDestinations>,
        local_broadcasts: Vec<GroupDestinations>,
        drops: Arc<AtomicU64>,
    ) -> Self {
        Self {
            groups,
            local_broadcasts,
            drops,
        }
    }

    /// What becomes of `npdu`, which this router would deliver on the port
    /// `route` names. A DADR that isn't a group there, or a route that
    /// isn't directly connected, is [`GroupDadr::No`]: the next router
    /// judges its own groups.
    pub(super) fn check(&self, route: &RouteEntry, npdu: &Npdu) -> GroupDadr {
        let Some(dadr) = npdu.destination.as_ref().map(|dest| &dest.mac_address[..]) else {
            return GroupDadr::No;
        };
        let on_port = |rules: &[GroupDestinations]| {
            rules
                .get(route.port_index)
                .is_some_and(|rule| rule.contains(dadr))
        };
        if !route.directly_connected || dadr.is_empty() || !on_port(&self.groups) {
            return GroupDadr::No;
        }
        let unconfirmed = !npdu.is_network_message
            && npdu.payload.first().is_some_and(|&first| {
                PduType::from_raw(first >> 4) == PduType::UNCONFIRMED_REQUEST
            });
        if unconfirmed && on_port(&self.local_broadcasts) {
            return GroupDadr::Broadcast;
        }
        count_drop(&self.drops);
        warn!(
            port = route.port_index,
            network_message = npdu.is_network_message,
            "Dropping a routed NPDU for a group DADR its port's broadcast doesn't stand for, \
             or that isn't an Unconfirmed-Request"
        );
        GroupDadr::Refused
    }
}
