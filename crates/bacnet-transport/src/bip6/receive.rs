//! The B/IPv6 receive loop: what happens to one datagram once the socket has
//! admitted it to the selected link.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_bip_codec::ipv6::decode;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::port::{ReceivedNpdu, TransportProvenance};
use crate::udp_metadata::ReceivedDatagram;

use super::frame::encoded;
use super::ingress::{
    destination_vmac_matches, forwarded_npdu_is_trusted, forwarded_source_is_usable,
    group_destination, original_destination_matches, LocalBinding,
};
use super::port::is_bip6_group;
use super::socket::Bip6Socket;
use super::vmac_table::VmacTable;
use super::{encode_bip6_mac, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error};

/// Octets an NPDU starts with whatever else it holds: version and control
/// (Clause 6.2).
const MIN_NPDU_LENGTH: usize = 2;

/// What the receive loop needs besides the datagram itself.
pub(super) struct Receiver {
    /// The transport's socket, also used for address-resolution replies.
    pub(super) socket: Arc<Bip6Socket>,
    /// Where NPDUs go up to the network layer.
    pub(super) tx: mpsc::Sender<ReceivedNpdu>,
    /// This node's B/IPv6 address as a MAC, to ignore its own frames.
    pub(super) local_mac: [u8; 18],
    /// This node's virtual MAC.
    pub(super) vmac: Bip6Vmac,
    /// The address the socket answers unicast on.
    pub(super) local_ip: Ipv6Addr,
    /// Unicast addresses of the local interfaces.
    pub(super) unicast_ips: Vec<Ipv6Addr>,
    /// Whether the socket is bound to the wildcard address.
    pub(super) wildcard_bind: bool,
    /// The BBMD this node is registered with as a foreign device, if any.
    pub(super) foreign_bbmd: Option<(Ipv6Addr, u16)>,
    /// Learned VMAC to address mappings.
    pub(super) vmac_table: VmacTable,
    /// Forwarded-NPDUs refused for a group origin (#1493), shared with the
    /// transport.
    pub(super) forwarded_group_origin_drops: Arc<AtomicU64>,
}

impl Receiver {
    /// Receive until the socket fails.
    pub(super) async fn run(self) {
        let mut recv_buf = vec![0u8; 2048];
        loop {
            match self.socket.recv_from(&mut recv_buf).await {
                Ok(received) => {
                    self.handle_datagram(&recv_buf[..received.len], &received)
                        .await;
                }
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    debug!(error = %e, "Dropping UDP datagram with invalid destination metadata");
                }
                Err(e) => {
                    warn!(error = %e, "IPv6 UDP recv error");
                    break;
                }
            }
        }
    }

    fn binding(&self) -> LocalBinding<'_> {
        LocalBinding {
            ip: self.local_ip,
            vmac: self.vmac,
            unicast_ips: &self.unicast_ips,
            wildcard_bind: self.wildcard_bind,
        }
    }

    /// Decode one datagram, drop it if its addressing does not fit its BVLC
    /// function, and otherwise act on it.
    pub(super) async fn handle_datagram(&self, data: &[u8], received: &ReceivedDatagram) {
        let message = match decode(data) {
            Ok(message) => message,
            Err(Bvll6Error::UnknownFunction { function }) => {
                debug!(function, "Unhandled BVLC6 function");
                return;
            }
            Err(e) => {
                warn!(error = %e, "Failed to decode BVLC6 frame");
                return;
            }
        };
        let function = message.function();
        if !destination_vmac_matches(&message, self.vmac) {
            debug!(
                function = function.to_raw(),
                destination_vmac = ?message.destination_vmac(),
                "Dropping BVLC6 message addressed to another VMAC"
            );
            return;
        }
        if !original_destination_matches(
            function,
            received.destination,
            message.destination_vmac(),
            &self.binding(),
            received.os_group_delivery,
        ) {
            debug!(
                function = function.to_raw(),
                destination = %received.destination,
                "Dropping BVLC6/IP or Destination-VMAC mismatch"
            );
            return;
        }
        if function == Bvlc6Function::FORWARDED_NPDU
            && !forwarded_npdu_is_trusted(
                received.peer,
                received.destination,
                received.os_group_delivery,
                self.local_ip,
                &self.unicast_ips,
                self.wildcard_bind,
                self.foreign_bbmd,
            )
        {
            debug!(
                peer = %received.peer,
                destination = %received.destination,
                "Dropping Forwarded-NPDU from an untrusted path"
            );
            return;
        }
        // Forwarded-NPDU's source VMAC identifies the original node, not the
        // forwarding BBMD. Its original address is learned from the function
        // payload in `dispatch`.
        if matches!(
            function,
            Bvlc6Function::ORIGINAL_UNICAST_NPDU
                | Bvlc6Function::ORIGINAL_BROADCAST_NPDU
                | Bvlc6Function::ADDRESS_RESOLUTION
                | Bvlc6Function::ADDRESS_RESOLUTION_ACK
                | Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION
                | Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
        ) {
            if let SocketAddr::V6(v6) = received.peer {
                self.vmac_table.learn(message.source_vmac(), v6).await;
            }
        }
        self.dispatch(message, received).await;
    }

    /// Send an address-resolution answer back to the peer that asked.
    async fn answer(&self, reply: &Bvll6<'_>, received: &ReceivedDatagram) {
        match encoded(reply) {
            Ok(reply) => {
                let _ = self.socket.send_to(&reply, received.peer).await;
            }
            Err(e) => debug!(error = %e, "Failed to encode BVLC6 reply"),
        }
    }

    /// Act on a message whose addressing `handle_datagram` has checked.
    pub(super) async fn dispatch(&self, message: Bvll6<'_>, received: &ReceivedDatagram) {
        match message {
            Bvll6::OriginalUnicastNpdu { npdu, .. } | Bvll6::OriginalBroadcastNpdu { npdu, .. } => {
                let SocketAddr::V6(v6) = received.peer else {
                    return;
                };
                let source_mac = MacAddr::from_slice(&encode_bip6_mac(*v6.ip(), v6.port()));
                if source_mac[..] == self.local_mac[..] {
                    return;
                }
                // A group delivery if either the function or the address
                // says so: an Original-Unicast-NPDU sent to a multicast group
                // reached every node in it, so a confirmed request in it is
                // never answered as a directed one (Clause 5.4.5.1).
                let link_layer_group = matches!(message, Bvll6::OriginalBroadcastNpdu { .. })
                    || group_destination(received.destination, received.os_group_delivery);
                if self
                    .tx
                    .try_send(ReceivedNpdu {
                        direct_response: None,
                        npdu: Bytes::copy_from_slice(npdu),
                        source_mac,
                        link_layer_group,
                        data_attributes: Vec::new(),
                        provenance: TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP6: NPDU channel full, dropping incoming frame");
                }
            }

            Bvll6::ForwardedNpdu {
                original_source,
                original_address,
                npdu,
            } => {
                // Too short to start an NPDU: dropped before anything else,
                // the origin included, is looked at.
                if npdu.len() < MIN_NPDU_LENGTH {
                    debug!(
                        length = npdu.len(),
                        "Dropping Forwarded-NPDU too short to hold an NPDU"
                    );
                    return;
                }
                // The 18 octets carry no scope ID, so the origin gets none.
                let source_addr = SocketAddrV6::from(original_address);
                // The origin is taken as the NPDU's source, so a group there
                // would bind a forged I-Am to every node in it, or send a
                // request's answer to them all. No node sends from one: the
                // frame is malformed, and counted (#1493).
                let source_mac = original_address.to_octets();
                if is_bip6_group(&source_mac) {
                    self.forwarded_group_origin_drops
                        .fetch_add(1, Ordering::Relaxed);
                    debug!(
                        source = %source_addr,
                        "Dropping Forwarded-NPDU whose origin is a group address"
                    );
                    return;
                }
                if !forwarded_source_is_usable(source_addr) {
                    debug!(
                        source = %source_addr,
                        "Dropping Forwarded-NPDU with unusable origin"
                    );
                    return;
                }
                let npdu = Bytes::copy_from_slice(npdu);
                if decode_npdu(npdu.clone()).is_err() {
                    debug!("Dropping Forwarded-NPDU with malformed NPDU");
                    return;
                }
                self.vmac_table.learn(original_source, source_addr).await;
                if self
                    .tx
                    .try_send(ReceivedNpdu {
                        direct_response: None,
                        npdu,
                        source_mac: MacAddr::from_slice(&source_mac),
                        link_layer_group: true,
                        data_attributes: Vec::new(),
                        provenance: TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP6: NPDU channel full, dropping forwarded frame");
                }
            }

            Bvll6::VirtualAddressResolution { source } => {
                // A node receiving VAR at its unicast B/IPv6 address
                // answers with its own VMAC and the requester's VMAC.
                let ack = Bvll6::VirtualAddressResolutionAck {
                    source: self.vmac,
                    destination: source,
                };
                self.answer(&ack, received).await;
            }

            Bvll6::AddressResolution { source, target } => {
                // AR: sender wants to know our B/IPv6 address from our VMAC.
                if target == self.vmac {
                    debug!(vmac = ?self.vmac, "Received AR for our VMAC, sending AR-Ack");
                    let ack = Bvll6::AddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    };
                    self.answer(&ack, received).await;
                }
            }

            Bvll6::AddressResolutionAck { source, .. } => {
                // AR-ACK: learn the sender's VMAC→address mapping
                // (will be used by VMAC table in future)
                debug!(
                    vmac = ?source,
                    addr = %received.peer,
                    "Received AR-Ack"
                );
            }

            Bvll6::VirtualAddressResolutionAck { source, .. } => {
                // VAR-ACK: someone responded to our collision check
                if source == self.vmac {
                    warn!(
                        vmac = ?self.vmac,
                        "BIP6 VMAC collision detected! \
                         Another node responded with our VMAC."
                    );
                }
            }

            Bvll6::BvlcResult { code, .. } => {
                // Log BVLC-Result for diagnostics
                if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
                    debug!("BIP6: BVLC-Result successful");
                } else {
                    tracing::error!(code = code.to_raw(), "BIP6: BVLC-Result NAK");
                }
            }

            // A node that isn't a BBMD should NAK a registration, an FDT
            // entry deletion or a broadcast distribution; here they go
            // unanswered: #1632.
            _ => {
                debug!(function = ?message.function(), "Unhandled BVLC6 function");
            }
        }
    }
}
