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
    destination_vmac_matches, forwarded_address_resolution_is_trusted, forwarded_npdu_is_trusted,
    forwarded_source_is_usable, group_destination, is_answerable_source,
    original_destination_matches, LocalBinding,
};
use super::port::is_bip6_group;
use super::reply_limit::ReplyLimiter;
use super::socket::Bip6Socket;
use super::vmac_table::VmacTable;
use super::{encode_bip6_mac, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error};

/// Octets an NPDU starts with whatever else it holds: version and control
/// (Clause 6.2).
const MIN_NPDU_LENGTH: usize = 2;

/// A BVLL message the receive loop owes, and the address it goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reply {
    /// Where the reply goes.
    pub(super) to: SocketAddr,
    /// The reply. No reply carries an NPDU, so none borrows the datagram
    /// it answers.
    pub(super) message: Bvll6<'static>,
}

/// What the receive loop needs besides the datagram itself.
pub(super) struct Receiver {
    /// The transport's socket, also used for the replies the loop sends.
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
    /// The quota the NAKs for BBMD requests count against.
    pub(super) nak_quota: ReplyLimiter,
    /// The quota the Address-Resolution-ACKs answering a
    /// Forwarded-Address-Resolution count against. It is separate from
    /// `nak_quota`, so a flood of BBMD requests, which anyone can send from
    /// many addresses, can't spend the budget these answers need.
    pub(super) far_ack_quota: ReplyLimiter,
}

impl Receiver {
    /// Receive until the socket fails.
    pub(super) async fn run(self) {
        let mut recv_buf = vec![0u8; 2048];
        loop {
            match self.socket.recv_from(&mut recv_buf).await {
                Ok(received) => {
                    let reply = self
                        .handle_datagram(&recv_buf[..received.len], &received)
                        .await;
                    if let Some(reply) = reply {
                        self.send(reply).await;
                    }
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

    /// Decode one datagram, drop it if its addressing or its sender doesn't
    /// fit its BVLC function, and otherwise act on it. Returns the reply it
    /// calls for, which the receive loop sends.
    pub(super) async fn handle_datagram(
        &self,
        data: &[u8],
        received: &ReceivedDatagram,
    ) -> Option<Reply> {
        let message = match decode(data) {
            Ok(message) => message,
            Err(Bvll6Error::UnknownFunction { function }) => {
                debug!(function, "Unhandled BVLC6 function");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Failed to decode BVLC6 frame");
                return None;
            }
        };
        let function = message.function();
        if !destination_vmac_matches(&message, self.vmac) {
            debug!(
                function = function.to_raw(),
                destination_vmac = ?message.destination_vmac(),
                "Dropping BVLC6 message addressed to another VMAC"
            );
            return None;
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
            return None;
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
            return None;
        }
        // A Forwarded-Address-Resolution names the address its answer goes
        // to, so the path it came by decides whether it is answered.
        if function == Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION
            && !forwarded_address_resolution_is_trusted(
                received.peer,
                received.destination,
                received.os_group_delivery,
                &self.binding(),
                self.foreign_bbmd,
            )
        {
            debug!(
                peer = %received.peer,
                destination = %received.destination,
                "Dropping Forwarded-Address-Resolution from an untrusted path"
            );
            return None;
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
        self.dispatch(message, received).await
    }

    /// Send a reply that `handle_datagram` returned.
    async fn send(&self, reply: Reply) {
        match encoded(&reply.message) {
            Ok(frame) => {
                if let Err(e) = self.socket.send_to(&frame, reply.to).await {
                    debug!(error = %e, to = %reply.to, "Failed to send BVLC6 reply");
                }
            }
            Err(e) => debug!(error = %e, "Failed to encode BVLC6 reply"),
        }
    }

    /// `message` for `to`, unless `quota` for `to` is spent.
    fn limited(
        &self,
        quota: &ReplyLimiter,
        to: SocketAddrV6,
        message: Bvll6<'static>,
    ) -> Option<Reply> {
        if !quota.admit_now(*to.ip()) {
            debug!(
                to = %to,
                function = message.function().to_raw(),
                "Dropping a BVLC6 reply over the reply quota"
            );
            return None;
        }
        Some(Reply {
            to: to.into(),
            message,
        })
    }

    /// Act on a message whose addressing and sender `handle_datagram` has
    /// checked, and return the reply it calls for.
    ///
    /// This transport is never a B/IPv6 BBMD. Annex U's receive-side rules
    /// for a node that isn't one, against what the transport does. "Usable
    /// source" means one [`is_answerable_source`] admits.
    ///
    /// | Message | Annex U asks | This transport | Clause |
    /// | --- | --- | --- | --- |
    /// | BVLC-Result | No reply: it reports on a request this node sent | Logged, a NAK at error level | U.2.1 |
    /// | Original-Unicast-NPDU | Take the NPDU when the destination VMAC is this node's, else discard | Handed up when sent to this node's address and VMAC | U.2.2, U.3 |
    /// | Original-Broadcast-NPDU | Take the NPDU | Handed up when sent to a BACnet multicast group | U.2.3, U.4 |
    /// | Address-Resolution | When it targets this node's VMAC, unicast an Address-Resolution-ACK to the asker | Answered at a usable source when sent to a BACnet group and targeting this VMAC; no quota | U.2.4, U.5 |
    /// | Forwarded-Address-Resolution | When it targets this node's VMAC, unicast an Address-Resolution-ACK to the asker's address it carries | Taken by a foreign device only by unicast from its configured BBMD, and by any other node only from a BACnet group with a usable source. Answered at the carried address when the target is this VMAC and that address is usable; the asker is then learned. FAR-answer quota | U.2.5, U.4.4, U.5 |
    /// | Address-Resolution-ACK | Update the VMAC table; discard one for another VMAC | Learned when sent to this node's address and VMAC | U.2.6, U.3, U.5 |
    /// | Virtual-Address-Resolution | Unicast a Virtual-Address-Resolution-ACK to the asker | Answered at a usable source when sent to this node's address; no quota | U.2.7, U.5 |
    /// | Virtual-Address-Resolution-ACK | Update the VMAC table; discard one for another VMAC | Learned when sent to this node's address and VMAC; one naming this node's VMAC as its source is logged as a collision | U.2.8, U.3, U.5 |
    /// | Forwarded-NPDU | Take the NPDU as from its original source | Handed up from a BACnet group, or in foreign mode from the configured BBMD, when the original address is usable; that address is learned | U.2.9, U.4.4 |
    /// | Register-Foreign-Device | Reply with its NAK | Its NAK at a usable source other than this node's own address, when sent to this node's address. NAK quota | U.4.5.3 |
    /// | Delete-Foreign-Device-Table-Entry | Reply with its NAK | The same | U.4.5.4 |
    /// | Distribute-Broadcast-To-Network | Reply with its NAK | The same | U.4.4 |
    /// | Function 0x0B | Nothing: its clause was removed | Dropped as unknown before dispatch | U.2.12 |
    ///
    /// The quotas are two [`ReplyLimiter`]s. A reply goes either to the
    /// datagram's source or, for a Forwarded-Address-Resolution that came by
    /// a trusted path ([`forwarded_address_resolution_is_trusted`]), to the
    /// asker's address that message carries; never anywhere else.
    pub(super) async fn dispatch(
        &self,
        message: Bvll6<'_>,
        received: &ReceivedDatagram,
    ) -> Option<Reply> {
        match message {
            Bvll6::OriginalUnicastNpdu { npdu, .. } | Bvll6::OriginalBroadcastNpdu { npdu, .. } => {
                let SocketAddr::V6(v6) = received.peer else {
                    return None;
                };
                let source_mac = MacAddr::from_slice(&encode_bip6_mac(*v6.ip(), v6.port()));
                if source_mac[..] == self.local_mac[..] {
                    return None;
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
                None
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
                    return None;
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
                    return None;
                }
                if !forwarded_source_is_usable(source_addr) {
                    debug!(
                        source = %source_addr,
                        "Dropping Forwarded-NPDU with unusable origin"
                    );
                    return None;
                }
                let npdu = Bytes::copy_from_slice(npdu);
                if decode_npdu(npdu.clone()).is_err() {
                    debug!("Dropping Forwarded-NPDU with malformed NPDU");
                    return None;
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
                None
            }

            Bvll6::VirtualAddressResolution { source } => {
                // A node receiving VAR at its unicast B/IPv6 address
                // answers with its own VMAC and the requester's VMAC.
                Some(Reply {
                    to: reply_address(received.peer)?.into(),
                    message: Bvll6::VirtualAddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    },
                })
            }

            Bvll6::AddressResolution { source, target } => {
                // AR: sender wants to know our B/IPv6 address from our VMAC.
                // Outside the reply quotas: a flood must not stop this node
                // answering another node's VMAC collision probe, which is
                // an AR for the prober's own VMAC.
                if target != self.vmac {
                    return None;
                }
                debug!(vmac = ?self.vmac, "Received AR for our VMAC, sending AR-Ack");
                Some(Reply {
                    to: reply_address(received.peer)?.into(),
                    message: Bvll6::AddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    },
                })
            }

            Bvll6::ForwardedAddressResolution {
                original_source,
                target,
                original_address,
            } => {
                if target != self.vmac {
                    return None;
                }
                // The answer goes to the asker, at the address the message
                // carries. That address has no scope ID, so it must be one
                // usable without one, as a Forwarded-NPDU's origin must.
                let asker = SocketAddrV6::from(original_address);
                if !forwarded_source_is_usable(asker) {
                    debug!(
                        asker = %asker,
                        "Dropping Forwarded-Address-Resolution with an unusable original address"
                    );
                    return None;
                }
                // The VMAC table follows incoming messages (U.5). The asker
                // is learned under the same guards as the answer (a trusted
                // path, this node's VMAC as the target, a usable address),
                // even when the quota then holds the answer back.
                self.vmac_table.learn(original_source, asker).await;
                debug!(
                    vmac = ?self.vmac,
                    asker = %asker,
                    "Received FAR for our VMAC, sending AR-Ack"
                );
                let ack = Bvll6::AddressResolutionAck {
                    source: self.vmac,
                    destination: original_source,
                };
                self.limited(&self.far_ack_quota, asker, ack)
            }

            Bvll6::AddressResolutionAck { source, .. } => {
                // `handle_datagram` learned the sender's address.
                debug!(
                    vmac = ?source,
                    addr = %received.peer,
                    "Received AR-Ack"
                );
                None
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
                None
            }

            Bvll6::BvlcResult { code, .. } => {
                // Log BVLC-Result for diagnostics
                if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
                    debug!("BIP6: BVLC-Result successful");
                } else {
                    tracing::error!(code = code.to_raw(), "BIP6: BVLC-Result NAK");
                }
                None
            }

            // Requests only a BBMD carries out. This node isn't one, so each
            // gets its own NAK (U.4.4, U.4.5.3, U.4.5.4).
            Bvll6::RegisterForeignDevice { .. }
            | Bvll6::DeleteForeignDeviceTableEntry { .. }
            | Bvll6::DistributeBroadcastToNetwork { .. } => {
                let code = message.function().nak()?;
                let source = reply_address(received.peer)?;
                // This node never sends itself one, so a request from its own
                // address is forged; refusing it would only NAK this node.
                if encode_bip6_mac(*source.ip(), source.port()) == self.local_mac {
                    debug!(
                        function = message.function().to_raw(),
                        "Dropping a BBMD request from this node's own address"
                    );
                    return None;
                }
                debug!(
                    function = message.function().to_raw(),
                    peer = %source,
                    "Refusing a BBMD request: this node is not a BBMD"
                );
                let nak = Bvll6::BvlcResult {
                    source: self.vmac,
                    code,
                };
                self.limited(&self.nak_quota, source, nak)
            }
        }
    }
}

/// The address to answer a request at: the datagram's source, unless an
/// answer there could reach more than one node or none
/// ([`is_answerable_source`]).
fn reply_address(peer: SocketAddr) -> Option<SocketAddrV6> {
    match peer {
        SocketAddr::V6(source) if is_answerable_source(peer) => Some(source),
        _ => {
            debug!(peer = %peer, "Not answering a request from an address no node sends from");
            None
        }
    }
}
