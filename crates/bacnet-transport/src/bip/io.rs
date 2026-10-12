use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_bip_codec::{
    BdtEntries, BvlcFunction, BvlcResultCode, Bvll, BvllError, BvllFrame, FdtEntries,
};
use bacnet_bip_core::bbmd::ForwardRefusal;
use bacnet_bip_core::Arrival;
use bacnet_types::MacAddr;

use crate::bbmd::{self, BbmdState, Forward};
use crate::bvll::{encode_bip_mac, encode_bvll, BvllMessage};
use crate::npdu_channel::{self, Refused};
use crate::port::ReceivedNpdu;

use super::fanout::FanoutDispatcher;
use super::ingress::Delivery;
use super::rate_limit::{is_covered_management_request, ManagementRateLimiter};
use super::receive_counters::{DropReason, ReceiveDrops};

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
pub(super) struct RecvContext {
    pub(super) local_mac: [u8; 6],
    pub(super) socket: Arc<super::BipSocket>,
    pub(super) npdu_tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) bbmd: Option<Arc<std::sync::Mutex<BbmdState>>>,
    pub(super) broadcast_addr: Ipv4Addr,
    pub(super) broadcast_port: u16,
    pub(super) client_management: Arc<super::client_management::ManagementClient>,
    pub(super) management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    pub(super) fanout: Option<FanoutDispatcher>,
    /// Refuses a UDP source (#1504) or a Forwarded-NPDU origin (#1493)
    /// that is a group address.
    pub(super) group_sources: super::groups::GroupSources,
    /// Where the datagrams this loop drops are counted (#1624), shared with
    /// the transport.
    pub(super) drops: Arc<ReceiveDrops>,
    #[cfg(test)]
    pub(super) force_dbtn_forward_failure: bool,
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
pub(super) async fn handle_bvll_message(
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
                // on this subnet, already reached it; never rebroadcasting
                // one that came by broadcast also ends any loop through a BDT
                // row that is this BBMD under another address.
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
                        // The own-echo check above makes OwnEcho unreachable;
                        // a Forwarded-NPDU is never refused as unregistered.
                        ctx.drops.count(match refusal {
                            ForwardRefusal::OwnEcho => DropReason::OwnEcho,
                            ForwardRefusal::NotFromPeer | ForwardRefusal::NotRegistered => {
                                DropReason::ForwardedNotFromPeer
                            }
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
            let code = refusal.result_code().unwrap_or(nak);
            send_bvlc_result(&ctx.socket, sender, code).await;
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
async fn send_bvlc_result(socket: &UdpSocket, dest: ([u8; 4], u16), code: BvlcResultCode) {
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
