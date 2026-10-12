//! The B/IP receive path as it was before the node engine, kept verbatim
//! for the cutover's differential test (#1622): `ingress.rs`, `io.rs` and
//! `groups.rs` as of the base. Only paths changed, and the BVLC-Result send
//! takes the socket wrapper, so a test can record it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bacnet_bip_codec::{
    address_from_slice, BdtEntries, BvlcFunction, BvlcResultCode, Bvll, BvllError, BvllFrame,
    FdtEntries,
};
use bacnet_bip_core::bbmd::ForwardRefusal;
use bacnet_bip_core::Arrival;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::bbmd::{self, BbmdState, Forward};
use crate::bip::fanout::FanoutDispatcher;
use crate::bip::rate_limit::ManagementRateLimiter;
use crate::bip::receive_counters::{DropReason, ReceiveDrops};
use crate::bip::BipSocket;
use crate::bvll::{encode_bip_mac, encode_bvll, BvllMessage};
use crate::npdu_channel::{self, Refused};
use crate::port::ReceivedNpdu;
use crate::udp_metadata::ReceivedDatagram;

use super::client::ManagementClient;

/// Which of a transport's sockets a datagram arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArrivalSocket {
    /// The socket every send leaves from.
    Primary,
    /// A receive-only broadcast listener.
    BroadcastListener,
}

/// Whether an inbound BVLC function shares the combined management quota
/// (`rate_limit.rs` as of the base).
pub(crate) fn is_covered_management_request(function: BvlcFunction) -> bool {
    function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
        || function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE
        || function == BvlcFunction::REGISTER_FOREIGN_DEVICE
        || function == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
}

/// How a datagram reached the B/IP socket, from its destination address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Sent to the configured broadcast address or 255.255.255.255, so every
    /// B/IP device on the subnet received it too.
    Broadcast,
    /// Any other destination.
    Unicast,
}

impl Delivery {
    /// Classify a datagram by its destination. An OS that flags the datagram
    /// as unicast delivery overrides a broadcast destination.
    pub(crate) fn of(
        destination: IpAddr,
        configured_broadcast: Ipv4Addr,
        os_group_delivery: Option<bool>,
    ) -> Self {
        let broadcast = matches!(destination, IpAddr::V4(ip) if ip == configured_broadcast || ip == Ipv4Addr::BROADCAST)
            && os_group_delivery != Some(false);
        if broadcast {
            Self::Broadcast
        } else {
            Self::Unicast
        }
    }
}

/// How an inbound datagram arrived, or `None` when its BVLC function does not
/// fit the address it was sent to.
///
/// Annex J tells the two Original functions apart by where they are sent. An
/// Original-Unicast-NPDU carries a directed NPDU to one node's own address
/// (J.2.11, J.3); an Original-Broadcast-NPDU carries a local broadcast, sent to
/// the subnet's broadcast address (J.4.1). A datagram that mixes the two is
/// dropped here, before its NPDU reaches the network layer.
/// An Original-Unicast-NPDU sent to a broadcast address reached every B/IP
/// node on the subnet, so a confirmed request in it would draw an answer from
/// each of them.
///
/// Where an interface bind's configured broadcast address is also its own
/// address, as in a loopback test, the BVLC function picks the reading. A
/// wildcard bind never counts the configured broadcast address as its own, so
/// there an Original-Unicast-NPDU sent to it is dropped. The handler reports
/// the delivery returned here as an Original-Unicast-NPDU's
/// `link_layer_group`, so the flag and this check cannot disagree.
pub(crate) fn admitted_delivery(
    function: BvlcFunction,
    destination: IpAddr,
    local_ip: Ipv4Addr,
    configured_broadcast: Ipv4Addr,
    local_unicast_ips: &[Ipv4Addr],
    wildcard_bind: bool,
    os_group_delivery: Option<bool>,
) -> Option<Delivery> {
    // A wildcard bind accepts unicast only to one of the host's listed
    // addresses, on every OS (#952).
    let local_unicast = match destination {
        IpAddr::V4(ip) if wildcard_bind => {
            ip != configured_broadcast
                && ip != Ipv4Addr::BROADCAST
                && !ip.is_multicast()
                && local_unicast_ips.contains(&ip)
        }
        IpAddr::V4(ip) => ip == local_ip,
        IpAddr::V6(_) => false,
    } && os_group_delivery != Some(true);
    let unicast = local_unicast.then_some(Delivery::Unicast);
    let broadcast = (Delivery::of(destination, configured_broadcast, os_group_delivery)
        == Delivery::Broadcast)
        .then_some(Delivery::Broadcast);

    match function {
        f if f == BvlcFunction::ORIGINAL_UNICAST_NPDU => unicast,
        f if f == BvlcFunction::ORIGINAL_BROADCAST_NPDU => broadcast,
        // A Forwarded-NPDU may arrive by direct unicast or by a configured
        // directed/limited broadcast, and counts as a broadcast when the
        // address fits both. All BVLL management requests and responses are
        // point-to-point and must arrive as actual unicast.
        f if f == BvlcFunction::FORWARDED_NPDU => broadcast.or(unicast),
        _ => unicast,
    }
}

/// This node's own addresses, which an inbound datagram's destination is
/// judged against.
pub(crate) struct IngressAddresses {
    /// The IP address in this node's B/IP MAC.
    pub(crate) local_ip: Ipv4Addr,
    /// The host's IPv4 addresses, which a wildcard bind takes unicast to.
    pub(crate) unicast_ips: Vec<Ipv4Addr>,
    /// Whether the socket is bound to 0.0.0.0 rather than an interface.
    pub(crate) wildcard_bind: bool,
    /// In per-address mode, the index of the interface the transport is
    /// bound to, when the host reports it. The broadcast listeners keep only
    /// what arrived on it.
    pub(crate) listener_interface: Option<u32>,
    /// Set by the first broadcast dropped for arriving on another interface,
    /// which is logged as a warning; the rest are logged at debug.
    pub(crate) interface_mismatch_seen: AtomicBool,
}

/// Whether a datagram a broadcast listener read came in on another
/// interface than the transport's (#1538). A listener bound to the limited
/// broadcast (Linux) or the wildcard address (macOS and the BSDs) gets the
/// broadcasts of every interface on the port, and one sent on another
/// network is not this network's. With either index unknown (or zero, which
/// names no interface), it stays.
pub(crate) fn arrived_elsewhere(listener_interface: Option<u32>, arrival: Option<u32>) -> bool {
    matches!((listener_interface, arrival), (Some(own), Some(arrival)) if arrival != 0 && own != arrival)
}

/// Whether a broadcast listener keeps a datagram: one sent to this link's
/// broadcast address that came in on the transport's own interface. Unicast
/// to a local address no socket on the port is bound to also reaches a
/// wildcard listener (macOS and the BSDs), and a listener gets the
/// broadcasts of every interface on the port (#1538). Neither is this
/// transport's, and another node on this port may share the listener, so
/// whatever such a datagram holds, it is counted and dropped before its
/// frame is read.
fn listener_keeps(
    received: &ReceivedDatagram,
    local: &IngressAddresses,
    ctx: &RecvContext,
) -> bool {
    let delivery = Delivery::of(
        received.destination,
        ctx.broadcast_addr,
        received.os_group_delivery,
    );
    if delivery != Delivery::Broadcast {
        ctx.drops.count(DropReason::ListenerMismatch);
        debug!(
            destination = %received.destination,
            "Dropping a datagram not sent to this link's broadcast address on the B/IP broadcast listener"
        );
        return false;
    }
    let (own, arrival) = (local.listener_interface, received.arrival_index);
    if !arrived_elsewhere(own, arrival) {
        return true;
    }
    ctx.drops.count(DropReason::ListenerMismatch);
    if local.interface_mismatch_seen.swap(true, Ordering::Relaxed) {
        debug!(
            ?own,
            ?arrival,
            "Dropping a broadcast that arrived on another interface"
        );
    } else {
        // The index was resolved at start; an interface that came back
        // with a new one needs a restart.
        warn!(
            ?own,
            ?arrival,
            "Dropping a broadcast that arrived on another interface than the B/IP \
             transport's; if its interface changed since start, restart the transport"
        );
    }
    false
}

/// Decode one received datagram and hand it to the BVLL handler, unless its
/// UDP source is a group address, a broadcast listener doesn't keep it, or
/// its BVLC function does not fit its destination.
pub(crate) async fn handle_datagram(
    data: &[u8],
    received: &ReceivedDatagram,
    arrival: ArrivalSocket,
    local: &IngressAddresses,
    ctx: &RecvContext,
) {
    // An IPv4 socket reports only IPv4 peers: any other is unusable
    // metadata, as in `receive_failed`.
    let SocketAddr::V4(peer) = received.peer else {
        ctx.drops.count(DropReason::ReceiveError);
        return;
    };
    // A group source goes before anything in the frame is read (#1504).
    if !ctx.group_sources.admits_sender(peer) {
        return;
    }
    if arrival == ArrivalSocket::BroadcastListener && !listener_keeps(received, local, ctx) {
        return;
    }
    // Per datagram, so at debug: a flood of them must not flood the log at
    // warn. The counters carry the signal (#1624).
    let frame = match BvllFrame::decode(data) {
        Ok(frame) => frame,
        Err(e) => {
            ctx.drops.count(DropReason::of_decode_error(&e));
            debug!(error = %e, "Failed to decode BVLL frame");
            return;
        }
    };
    let Some(delivery) = admitted_delivery(
        frame.function,
        received.destination,
        local.local_ip,
        ctx.broadcast_addr,
        &local.unicast_ips,
        local.wildcard_bind,
        received.os_group_delivery,
    ) else {
        ctx.drops.count(DropReason::DestinationMismatch);
        debug!(
            function = frame.function.to_raw(),
            destination = %received.destination,
            "Dropping BVLL/IP destination mismatch"
        );
        return;
    };
    // Where the configured broadcast address is also the interface's own,
    // as in a loopback test, a unicast function sent to it passes both
    // checks above as a unicast; the listener still keeps broadcasts only.
    if arrival == ArrivalSocket::BroadcastListener && delivery != Delivery::Broadcast {
        ctx.drops.count(DropReason::ListenerMismatch);
        debug!(
            destination = %received.destination,
            "Dropping unicast on the B/IP broadcast listener"
        );
        return;
    }
    handle_bvll_message(frame, (peer.ip().octets(), peer.port()), delivery, ctx).await;
}

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
pub(crate) struct RecvContext {
    pub(crate) local_mac: [u8; 6],
    pub(crate) socket: Arc<BipSocket>,
    pub(crate) npdu_tx: mpsc::Sender<ReceivedNpdu>,
    pub(crate) bbmd: Option<Arc<std::sync::Mutex<BbmdState>>>,
    pub(crate) broadcast_addr: Ipv4Addr,
    pub(crate) broadcast_port: u16,
    pub(crate) client_management: Arc<ManagementClient>,
    pub(crate) management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    pub(crate) fanout: Option<FanoutDispatcher>,
    /// Refuses a UDP source (#1504) or a Forwarded-NPDU origin (#1493)
    /// that is a group address.
    pub(crate) group_sources: GroupSources,
    /// Where the datagrams this loop drops are counted (#1624), shared with
    /// the transport.
    pub(crate) drops: Arc<ReceiveDrops>,
    #[cfg(test)]
    pub(crate) force_dbtn_forward_failure: bool,
}

/// Hand a management response to the request waiting for it. The frame is
/// copied, since the request outlives the receive buffer.
fn complete_pending_bvlc_response(
    frame: BvllFrame<'_>,
    sender: ([u8; 4], u16),
    ctx: &RecvContext,
) -> bool {
    ctx.client_management
        .complete(&BvllMessage::from(frame), sender)
}

/// Hand an NPDU this transport received up to the network layer, or count
/// it dropped when the channel is full or closed. `frame` names its kind for
/// the log. The channel's slot is reserved before the NPDU is copied, so a
/// drop costs no allocation.
fn hand_up(
    ctx: &RecvContext,
    npdu: &[u8],
    source_mac: MacAddr,
    link_layer_group: bool,
    frame: &'static str,
) {
    let npdu = || Bytes::copy_from_slice(npdu);
    match npdu_channel::hand_up(&ctx.npdu_tx, npdu, source_mac, link_layer_group) {
        Ok(()) => {}
        Err(Refused::Full) => {
            ctx.drops.npdu_channel_full(frame);
        }
        Err(Refused::Closed) => {
            ctx.drops.count(DropReason::NpduChannelClosed);
            debug!(
                frame,
                "B/IP: NPDU channel closed, dropping an incoming frame"
            );
        }
    }
}

/// Whether `mac` is this node's own address, counting the frame as its own
/// echo when it is.
fn own_echo(mac: &[u8], ctx: &RecvContext) -> bool {
    let own = mac == ctx.local_mac;
    if own {
        ctx.drops.count(DropReason::OwnEcho);
    }
    own
}

/// Handle a BVLL frame whose header the codec accepted. `delivery` says how
/// its datagram arrived.
pub(crate) async fn handle_bvll_message(
    frame: BvllFrame<'_>,
    sender: ([u8; 4], u16),
    delivery: Delivery,
    ctx: &RecvContext,
) {
    // Bounded inbound management quota. Excess covered requests are
    // silently discarded before payload validation, table access, ACL
    // evaluation, or any response/NAK. Write-BDT and data-plane functions
    // are excluded and fall through below.
    if is_covered_management_request(frame.function) {
        let allowed = match ctx.management_limiter.lock() {
            Ok(mut limiter) => limiter.check_now(sender.0),
            Err(poison) => poison.into_inner().check_now(sender.0),
        };
        if !allowed {
            ctx.drops.count(DropReason::RateLimited);
            debug!(
                function = frame.function.to_raw(),
                ip = %Ipv4Addr::from(sender.0),
                "Discarding over-limit BBMD management request"
            );
            return;
        }
    }
    let message = match frame.message() {
        Ok(message) => message,
        Err(error) => return handle_malformed(frame, error, sender, ctx).await,
    };
    match message {
        Bvll::OriginalUnicastNpdu(npdu) => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if own_echo(&source_mac, ctx) {
                return;
            }
            // From the datagram's address, not its BVLC function, so a
            // group-addressed confirmed request is never answered as a
            // directed one (Clause 5.4.5.1).
            let group = delivery == Delivery::Broadcast;
            hand_up(ctx, npdu, source_mac, group, "unicast");
        }

        Bvll::OriginalBroadcastNpdu(npdu) => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if own_echo(&source_mac, ctx) {
                return;
            }

            hand_up(ctx, npdu, source_mac, true, "broadcast");

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = &ctx.bbmd {
                let source = socket_address(sender);
                let forward =
                    bbmd::lock(bbmd).forward_original_broadcast(source, npdu, Instant::now());
                let _ = forward_remote(ctx, npdu, forward).await;
            }
        }

        Bvll::ForwardedNpdu { origin, npdu } => {
            // The embedded origin is the NPDU's source, in BBMD mode and
            // out of it (Annex J.2.5).
            let (orig_ip, orig_port) = (origin.ip().octets(), origin.port());
            let source_mac = MacAddr::from(encode_bip_mac(orig_ip, orig_port));
            if own_echo(&source_mac, ctx) || !ctx.group_sources.admits_origin(&source_mac) {
                return;
            }

            // BBMD mode: only accept FORWARDED_NPDU from BDT peers
            if let Some(bbmd) = &ctx.bbmd {
                // A local rebroadcast retains the remote embedded origin. The
                // self BDT row must not turn its UDP echo into peer traffic.
                if own_echo(&encode_bip_mac(sender.0, sender.1), ctx) {
                    return;
                }
                // Only a BDT peer's is delivered and forwarded: to the
                // foreign devices (the peers had it from its first BBMD), and
                // to the local subnet when it came to this BBMD alone, which
                // this BBMD's own two-hop row and a unicast arrival tell
                // (Annex J.4.5). One that came by broadcast, or from a sender
                // on this subnet at this port, already reached it; never
                // rebroadcasting one that came by broadcast also ends any loop
                // through a BDT row that is this BBMD under another address.
                let arrival = match delivery {
                    Delivery::Broadcast => Arrival::Broadcast,
                    Delivery::Unicast => Arrival::Unicast,
                };
                let decision = bbmd::lock(bbmd).forward_forwarded_npdu(
                    socket_address(sender),
                    arrival,
                    origin,
                    local_broadcast(ctx),
                    Instant::now(),
                );
                let forward = match decision {
                    Ok(forward) => forward,
                    Err(refusal) => {
                        // The own-echo check above makes OwnEcho unreachable.
                        ctx.drops.count(match refusal {
                            ForwardRefusal::OwnEcho => DropReason::OwnEcho,
                            ForwardRefusal::NotFromPeer => DropReason::ForwardedNotFromPeer,
                        });
                        debug!(
                            "Rejecting FORWARDED_NPDU from non-BDT sender {:?}:{}",
                            Ipv4Addr::from(sender.0),
                            sender.1
                        );
                        return;
                    }
                };

                hand_up(ctx, npdu, source_mac, true, "forwarded");

                match forward.local_broadcast {
                    Some(local_dest) => {
                        let _ = send_forwarded_npdu(ctx, local_dest, origin, npdu).await;
                    }
                    None if delivery == Delivery::Broadcast => {
                        debug!("Forwarded-NPDU arrived by broadcast; no local rebroadcast");
                    }
                    None => {}
                }
                let _ = forward_remote(ctx, npdu, forward).await;
            } else {
                hand_up(ctx, npdu, source_mac, true, "forwarded");
            }
        }

        Bvll::DistributeBroadcastToNetwork(npdu) => {
            handle_distribute_broadcast(npdu, sender, ctx).await;
        }

        Bvll::RegisterForeignDevice { ttl } => {
            let result = match &ctx.bbmd {
                Some(bbmd) => {
                    let result = bbmd::lock(bbmd).register_foreign_device(sender.0, sender.1, ttl);
                    debug!(
                        ip = ?Ipv4Addr::from(sender.0),
                        port = sender.1,
                        ttl = ttl,
                        "Foreign device registered"
                    );
                    result
                }
                None => BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            };
            send_bvlc_result(&ctx.socket, sender, result).await;
        }

        Bvll::ReadBroadcastDistributionTable => match &ctx.bbmd {
            Some(bbmd) => {
                let rows = bbmd::lock(bbmd).bdt_wire_entries();
                let ack = Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&rows));
                send_table_ack(ack, sender, ctx).await;
            }
            None => {
                let nak = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
                send_bvlc_result(&ctx.socket, sender, nak).await;
            }
        },

        Bvll::WriteBroadcastDistributionTable(_) => {
            // Annex J.4.4.2 requires receivers to answer Write-BDT with the
            // not-supported result. No state change or persistence.
            let nak = BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK;
            send_bvlc_result(&ctx.socket, sender, nak).await;
        }

        Bvll::ReadForeignDeviceTable => match &ctx.bbmd {
            Some(bbmd) => {
                let rows = bbmd::lock(bbmd).fdt_wire_entries();
                let ack = Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&rows));
                send_table_ack(ack, sender, ctx).await;
            }
            None => {
                let nak = BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK;
                send_bvlc_result(&ctx.socket, sender, nak).await;
            }
        },

        Bvll::DeleteForeignDeviceTableEntry(entry) => {
            let nak = BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK;
            let result = match &ctx.bbmd {
                Some(bbmd) => {
                    // Check management ACL before accepting Delete-FDT-Entry
                    let mut state = bbmd::lock(bbmd);
                    if state.is_management_allowed(&sender.0) {
                        state.delete_foreign_device(entry.ip().octets(), entry.port())
                    } else {
                        debug!(
                            "Rejecting Delete-FDT-Entry from non-ACL sender {:?}:{}",
                            Ipv4Addr::from(sender.0),
                            sender.1
                        );
                        nak
                    }
                }
                None => nak,
            };
            send_bvlc_result(&ctx.socket, sender, result).await;
        }

        Bvll::BvlcResult(code) => {
            if !complete_pending_bvlc_response(frame, sender, ctx) {
                debug!(code = ?code, "Received unmatched BVLC-Result");
            }
        }

        Bvll::ReadBroadcastDistributionTableAck(_) => {
            if !complete_pending_bvlc_response(frame, sender, ctx) {
                debug!("Received Read-BDT-ACK with no pending request");
            }
        }

        Bvll::ReadForeignDeviceTableAck(_) => {
            if !complete_pending_bvlc_response(frame, sender, ctx) {
                debug!("Received Read-FDT-ACK with no pending request");
            }
        }
    }
}

/// A frame the codec accepted whose payload doesn't fit its function, or
/// whose function it doesn't know.
///
/// A malformed BBMD-only request is answered with its NAK, in every mode,
/// so the requester isn't left waiting: J.2.1 gives no other reply for a
/// request the receiver can't carry out. A malformed management response
/// still reaches the request waiting for it, which counts it and, for an
/// ACK, fails with the decode error. Anything else is dropped.
async fn handle_malformed(
    frame: BvllFrame<'_>,
    error: BvllError,
    sender: ([u8; 4], u16),
    ctx: &RecvContext,
) {
    ctx.drops.count(DropReason::of_decode_error(&error));
    if let Some(nak) = frame.function.nak() {
        debug!(error = %error, "Refusing a malformed BVLL management request");
        send_bvlc_result(&ctx.socket, sender, nak).await;
        return;
    }
    match frame.function {
        BvlcFunction::BVLC_RESULT
        | BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
        | BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
            if !complete_pending_bvlc_response(frame, sender, ctx) {
                debug!(error = %error, "Received malformed BVLC response");
            }
        }
        _ => debug!(
            function = frame.function.to_raw(),
            error = %error,
            "Dropping unknown or malformed BVLL message"
        ),
    }
}

/// A Distribute-Broadcast-To-Network (Annex J.4.5): a BBMD forwards it for a
/// registered foreign device, and every other device refuses it.
async fn handle_distribute_broadcast(npdu: &[u8], sender: ([u8; 4], u16), ctx: &RecvContext) {
    let nak = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;
    let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
    if own_echo(&source_mac, ctx) {
        return;
    }
    // Non-BBMD: reject with NAK (spec J.4.5)
    let Some(bbmd) = &ctx.bbmd else {
        send_bvlc_result(&ctx.socket, sender, nak).await;
        return;
    };

    // If BBMD, only a registered foreign device may distribute
    let decision = bbmd::lock(bbmd).forward_distribute_broadcast(
        socket_address(sender),
        npdu,
        local_broadcast(ctx),
        Instant::now(),
    );
    let forward = match decision {
        Ok(forward) => forward,
        Err(refusal) => {
            debug!(
                "Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender {:?}:{}",
                Ipv4Addr::from(sender.0),
                sender.1
            );
            send_bvlc_result(&ctx.socket, sender, refusal.result_code()).await;
            return;
        }
    };

    hand_up(ctx, npdu, source_mac, true, "distributed broadcast");

    let local_ok = if should_force_dbtn_forward_failure(ctx) {
        warn!("Forced DBTN forwarding failure");
        false
    } else {
        match forward.local_broadcast {
            Some(local_dest) => send_forwarded_npdu(ctx, local_dest, forward.origin, npdu).await,
            None => true,
        }
    };

    let remote_ok = forward_remote(ctx, npdu, forward).await;

    if !(local_ok && remote_ok) {
        send_bvlc_result(&ctx.socket, sender, nak).await;
    }
}

/// Send a Read-BDT-Ack or Read-FDT-Ack, unless it would spend more of the
/// requester's response-amplification budget than is left.
async fn send_table_ack(ack: Bvll<'_>, sender: ([u8; 4], u16), ctx: &RecvContext) {
    let resp_len = match ack.encoded_len() {
        Ok(length) => length,
        Err(error) => {
            warn!(error = %error, function = %ack.function(), "Failed to encode table ACK");
            return;
        }
    };
    let bdt = ack.function() == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK;
    let check = |limiter: &mut ManagementRateLimiter| {
        if bdt {
            limiter.check_and_record_bdt_response(sender.0, resp_len)
        } else {
            limiter.check_and_record_fdt_response(sender.0, resp_len)
        }
    };
    let allowed = match ctx.management_limiter.lock() {
        Ok(mut limiter) => check(&mut limiter),
        Err(poison) => check(&mut poison.into_inner()),
    };
    if !allowed {
        debug!(
            ip = %Ipv4Addr::from(sender.0),
            bytes = resp_len,
            function = %ack.function(),
            "Throttling table ACK due to amplification budget"
        );
        return;
    }
    let mut buf = BytesMut::with_capacity(resp_len);
    match encode_bvll(&mut buf, &ack) {
        Ok(()) => {
            let dest = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
            let _ = ctx.socket.send_to(&buf, dest).await;
        }
        Err(e) => warn!(error = %e, "Failed to encode table ACK"),
    }
}

/// Send a BVLC-Result to a destination.
async fn send_bvlc_result(socket: &BipSocket, dest: ([u8; 4], u16), code: BvlcResultCode) {
    let mut buf = BytesMut::with_capacity(6);
    if let Err(e) = encode_bvll(&mut buf, &Bvll::BvlcResult(code)) {
        warn!(error = %e, "Failed to encode BVLC-Result");
        return;
    }
    let addr = SocketAddrV4::new(Ipv4Addr::from(dest.0), dest.1);
    let _ = socket.send_to(&buf, addr).await;
}

#[cfg(test)]
fn should_force_dbtn_forward_failure(ctx: &RecvContext) -> bool {
    ctx.force_dbtn_forward_failure
}

#[cfg(not(test))]
fn should_force_dbtn_forward_failure(_ctx: &RecvContext) -> bool {
    false
}

/// `sender` as a socket address.
fn socket_address(sender: ([u8; 4], u16)) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1)
}

/// This subnet's B/IP broadcast address, where a BBMD rebroadcasts.
fn local_broadcast(ctx: &RecvContext) -> SocketAddrV4 {
    SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port)
}

/// Send `forward`'s peer and foreign-device destinations a Forwarded-NPDU
/// carrying `npdu`: through the fan-out worker's queue and budgets, or, in a
/// receive context without one, directly. Whether it was queued or sent.
async fn forward_remote(ctx: &RecvContext, npdu: &[u8], forward: Forward) -> bool {
    let origin = forward.origin;
    match &ctx.fanout {
        Some(fanout) => fanout.dispatch_forwarded_npdu(
            origin.ip().octets(),
            origin.port(),
            npdu,
            forward.remote,
            forward.deduplicated,
        ),
        None => forward_npdu(ctx, npdu, origin, &forward.remote).await,
    }
}

/// Encode a Forwarded-NPDU carrying `npdu` from `origin`.
fn forwarded_npdu(origin: SocketAddrV4, npdu: &[u8]) -> Option<BytesMut> {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    let message = Bvll::ForwardedNpdu { origin, npdu };
    match encode_bvll(&mut buf, &message) {
        Ok(()) => Some(buf),
        Err(e) => {
            warn!(error = %e, "Failed to encode Forwarded-NPDU");
            None
        }
    }
}

/// Send `npdu` from `origin` to `dest` as a Forwarded-NPDU. A failed send
/// is logged through the throttle in [`ReceiveDrops::forward_failed`].
async fn send_forwarded_npdu(
    ctx: &RecvContext,
    dest: SocketAddrV4,
    origin: SocketAddrV4,
    npdu: &[u8],
) -> bool {
    let Some(buf) = forwarded_npdu(origin, npdu) else {
        return false;
    };
    match ctx.socket.send_to(&buf, dest).await {
        Ok(_) => true,
        Err(e) => {
            ctx.drops.forward_failed(&e, dest);
            false
        }
    }
}

/// Forward an NPDU as Forwarded-NPDU to a list of targets, logging a failed
/// send as [`send_forwarded_npdu`] does. Only a receive context without a
/// fan-out worker, as in some tests, forwards this way.
///
/// Yields between sends for large target lists to avoid starving the recv loop
/// when there are many targets (up to 128 BDT rows and 128 foreign devices).
async fn forward_npdu(
    ctx: &RecvContext,
    npdu: &[u8],
    origin: SocketAddrV4,
    targets: &[SocketAddrV4],
) -> bool {
    if targets.is_empty() {
        return true;
    }
    let Some(buf) = forwarded_npdu(origin, npdu) else {
        return false;
    };
    let frame = buf.freeze();
    let mut all_sent = true;

    for (i, &dest) in targets.iter().enumerate() {
        if let Err(e) = ctx.socket.send_to(&frame, dest).await {
            all_sent = false;
            ctx.drops.forward_failed(&e, dest);
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
        }
    }
    all_sent
}

/// A B/IP link's group destinations, as [`TransportPort::is_group_destination`](crate::port::TransportPort::is_group_destination)
/// reads them (#1479): this link's broadcast MAC, the limited broadcast
/// 255.255.255.255 or the configured broadcast IP at any UDP port, and any
/// IPv4 multicast address (224.0.0.0/4). The socket may send to each, since
/// it sets SO_BROADCAST, and each reaches more than one node. A configured
/// broadcast IP that is a loopback address or this link's own interface,
/// as loopback tests set it up, counts only at this link's port, as
/// [`TransportPort::is_broadcast_mac`](crate::port::TransportPort::is_broadcast_mac) has it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BipGroups {
    broadcast: Ipv4Addr,
    port: u16,
    interface: Ipv4Addr,
}

impl BipGroups {
    /// The groups of a link bound to `interface` at `port` whose configured
    /// broadcast IP is `broadcast`.
    pub(crate) fn new(broadcast: Ipv4Addr, port: u16, interface: Ipv4Addr) -> Self {
        Self {
            broadcast,
            port,
            interface,
        }
    }

    pub(crate) fn contains(self, mac: &[u8]) -> bool {
        let Ok(address) = address_from_slice(mac) else {
            return false;
        };
        let ip = *address.ip();
        (ip == self.broadcast && address.port() == self.port) || self.is_group_ip(ip)
    }

    /// A group address whatever the port: the limited broadcast, a
    /// multicast address, or the configured broadcast IP unless it is one of
    /// this node's own addresses. Only that last case leaves
    /// [`Self::contains`] to the port, so this is the test for a sender: a
    /// datagram from this link's own address and port is this node's own.
    fn is_group_ip(self, ip: Ipv4Addr) -> bool {
        ip.is_broadcast()
            || ip.is_multicast()
            || (ip == self.broadcast && !ip.is_loopback() && ip != self.interface)
    }
}

/// The receive loop's checks on the addresses a datagram names as its
/// sender: its UDP source (#1504) and, in a Forwarded-NPDU, the originating
/// address (#1493). The stack takes the sender as the NPDU's source and
/// answers it there, and a BBMD registers it as a foreign device or forwards
/// from it. One that is a group address of this link ([`BipGroups`]) would
/// let a forged I-Am bind a device to a group, and send the answer to a
/// request, or a confirmed COV notification, to every node in it. No node
/// sends from such an address, so the datagram is dropped before any BVLC
/// function is handled, a BBMD forwards it nowhere, and each one is counted.
#[derive(Clone)]
pub(crate) struct GroupSources {
    groups: BipGroups,
    sender_drops: Arc<AtomicU64>,
    origin_drops: Arc<AtomicU64>,
}

impl GroupSources {
    /// Refuse `groups`, counting a refused UDP source in `sender_drops` and
    /// a refused Forwarded-NPDU origin in `origin_drops`.
    pub(crate) fn new(
        groups: BipGroups,
        sender_drops: Arc<AtomicU64>,
        origin_drops: Arc<AtomicU64>,
    ) -> Self {
        Self {
            groups,
            sender_drops,
            origin_drops,
        }
    }

    /// A rule for receive contexts built in tests that don't exercise it:
    /// the limited broadcast and multicast addresses, with its own counters.
    pub(crate) fn detached() -> Self {
        let unspecified = Ipv4Addr::UNSPECIFIED;
        let groups = BipGroups::new(unspecified, 0, unspecified);
        Self::new(groups, Arc::default(), Arc::default())
    }

    /// Whether a datagram from UDP source `sender` may go on. A group source
    /// is counted and logged, and the datagram goes no further.
    pub(crate) fn admits_sender(&self, sender: SocketAddrV4) -> bool {
        if !self.groups.is_group_ip(*sender.ip()) {
            return true;
        }
        self.sender_drops.fetch_add(1, Ordering::Relaxed);
        debug!(%sender, "Dropping a datagram whose UDP source is a group address");
        false
    }

    /// Whether a Forwarded-NPDU from `origin` (its B/IP MAC) may go on. A
    /// group origin is counted and logged, and the frame goes no further.
    pub(crate) fn admits_origin(&self, origin: &[u8]) -> bool {
        if !self.groups.contains(origin) {
            return true;
        }
        self.origin_drops.fetch_add(1, Ordering::Relaxed);
        debug!(
            ?origin,
            "Dropping Forwarded-NPDU whose origin is a group address"
        );
        false
    }
}
