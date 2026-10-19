//! The network-layer messages a non-routing node's user sends and reads
//! (Clause 6.4): a send for a local network message, and the reports the
//! user acts on, read from a received control with the one parser each
//! payload has (#1794, #1818).

use super::*;
use crate::network_number::NumberControl;
use bacnet_npdu::{NetworkMessageType, RejectMessageToNetwork};
use bacnet_router_core::message::{NetworkNumberIs, Networks};

/// A network message that tells a non-routing node something about the
/// internetwork, read from a [`ReceivedNetworkControl`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkReport {
    /// I-Am-Router-To-Network (Clause 6.4.2): the sender routes to these
    /// networks, in the order it listed them.
    IAmRouterToNetwork(Vec<u16>),
    /// Network-Number-Is (Clause 6.4.20): the number of the local network,
    /// and whether the sender had it configured rather than learned.
    NetworkNumberIs {
        /// The local network's number, in 1..=65534.
        network: u16,
        /// Whether the sender's number was configured (flag 1).
        configured: bool,
    },
    /// Reject-Message-To-Network (Clause 6.4.4): a router refused a message
    /// this node sent, for the reason and network it names.
    RejectMessageToNetwork(RejectMessageToNetwork),
}

/// A [`NetworkReport`] and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedNetworkReport {
    /// The link-layer peer that sent it: for an I-Am-Router-To-Network, the
    /// router to send through to reach the networks it lists.
    pub source_mac: MacAddr,
    /// SNET/SADR, when a router passed the message on from another network,
    /// as a reject from a router further along the path is.
    pub source_network: Option<NpduAddress>,
    /// What the message reports.
    pub report: NetworkReport,
}

impl ReceivedNetworkReport {
    /// Read `control` as a report, or `None` for any other message type or
    /// a payload its parser refuses.
    ///
    /// - I-Am-Router-To-Network: router-core's [`Networks`] view, with at
    ///   least one network and none of 0 or 0xFFFF, which name no network
    ///   a router can reach. A list with either is refused whole.
    /// - Network-Number-Is: [`NumberControl::parse`], the reader the local
    ///   Number controls use, so a routed, unicast or malformed one is
    ///   ignored here too (Clause 6.4.20).
    /// - Reject-Message-To-Network: [`RejectMessageToNetwork::decode`],
    ///   exactly three octets.
    pub fn parse(control: &ReceivedNetworkControl) -> Option<Self> {
        let npdu = &control.npdu;
        let report = match npdu.message_type? {
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK => {
                let networks = Networks::parse(&npdu.payload).ok()?;
                let listed: Vec<u16> = networks.iter().collect();
                if listed.is_empty() || listed.iter().any(|&n| n == 0 || n == u16::MAX) {
                    return None;
                }
                NetworkReport::IAmRouterToNetwork(listed)
            }
            NetworkMessageType::NETWORK_NUMBER_IS => match NumberControl::parse(control)? {
                NumberControl::NumberIs { number, flag } => NetworkReport::NetworkNumberIs {
                    network: number,
                    configured: flag == NetworkNumberIs::CONFIGURED,
                },
                NumberControl::WhatIs => return None,
            },
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK => NetworkReport::RejectMessageToNetwork(
                RejectMessageToNetwork::decode(&npdu.payload).ok()?,
            ),
            _ => return None,
        };
        Some(Self {
            source_mac: control.source_mac.clone(),
            source_network: npdu.source.clone(),
            report,
        })
    }
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Send a network-layer message on the local network, with no DNET:
    /// broadcast when `destination_mac` is `None`, else to that peer.
    ///
    /// The NPDU names no destination network, so routers act on it
    /// themselves rather than pass it on, as Who-Is-Router-To-Network and
    /// What-Is-Network-Number need (Clauses 6.4.1 and 6.4.19). A
    /// `destination_mac` that is one of the link's local broadcast
    /// destinations goes out as the local broadcast, as
    /// [`Self::send_apdu`] does. A proprietary message type, which needs a
    /// vendor ID this send doesn't take, is refused before anything is
    /// sent.
    pub async fn send_network_message(
        &self,
        message_type: NetworkMessageType,
        payload: &[u8],
        destination_mac: Option<&[u8]>,
    ) -> Result<(), Error> {
        if message_type.is_proprietary() {
            return Err(Error::Encoding(format!(
                "network message type {message_type} is proprietary and needs a vendor ID"
            )));
        }
        let npdu = Npdu {
            message_type: Some(message_type),
            payload: Bytes::copy_from_slice(payload),
            ..Npdu::default()
        };
        let mut buf = BytesMut::with_capacity(3 + payload.len());
        encode_npdu(&mut buf, &npdu)?;
        match destination_mac {
            Some(mac) if !self.local_broadcasts.contains(mac) => {
                self.transport.send_unicast(&buf, mac).await
            }
            _ => self.transport.send_broadcast(&buf).await,
        }
    }
}

#[cfg(test)]
#[path = "layer_network_message_tests.rs"]
mod tests;
