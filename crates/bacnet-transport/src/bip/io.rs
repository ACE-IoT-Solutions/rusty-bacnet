use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_bip_codec::{
    BdtEntries, BvlcFunction, BvlcResultCode, Bvll, BvllError, BvllFrame, FdtEntries,
};
use bacnet_types::MacAddr;

use crate::bbmd::{self, BbmdState};
use crate::bvll::{encode_bip_mac, encode_bvll, BvllMessage};
use crate::npdu_channel::{self, Refused};
use crate::port::{ReceivedNpdu, TransportProvenance};

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
    provenance: TransportProvenance,
    frame: &'static str,
) {
    let npdu = || Bytes::copy_from_slice(npdu);
    match npdu_channel::hand_up_with_provenance(
        &ctx.npdu_tx,
        npdu,
        source_mac,
        link_layer_group,
        provenance,
    ) {
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
            hand_up(
                ctx,
                npdu,
                source_mac,
                group,
                TransportProvenance::unverified_bip(
                    BvlcFunction::ORIGINAL_UNICAST_NPDU.to_raw(),
                    sender,
                    None,
                ),
                "unicast",
            );
        }

        Bvll::OriginalBroadcastNpdu(npdu) => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if own_echo(&source_mac, ctx) {
                return;
            }

            hand_up(
                ctx,
                npdu,
                source_mac,
                true,
                TransportProvenance::unverified_bip(
                    BvlcFunction::ORIGINAL_BROADCAST_NPDU.to_raw(),
                    sender,
                    None,
                ),
                "broadcast",
            );

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = &ctx.bbmd {
                let (targets, dedup_count) = {
                    let mut state = bbmd::lock(bbmd);
                    let before = state.fdt_counters().destinations_deduplicated;
                    let targets = state.forwarding_targets(sender.0, sender.1);
                    let after = state.fdt_counters().destinations_deduplicated;
                    (targets, after.saturating_sub(before))
                };
                if let Some(fanout) = &ctx.fanout {
                    let addrs = targets
                        .into_iter()
                        .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
                        .collect();
                    fanout.dispatch_forwarded_npdu(sender.0, sender.1, npdu, addrs, dedup_count);
                } else {
                    let _ = forward_npdu(ctx, npdu, sender.0, sender.1, &targets).await;
                }
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
                let (is_bdt_peer, needs_local_broadcast) = {
                    let state = bbmd::lock(bbmd);
                    (
                        state.is_bdt_peer(sender.0, sender.1),
                        state.forwarded_npdu_needs_local_broadcast(sender.0, sender.1),
                    )
                };
                if !is_bdt_peer {
                    ctx.drops.count(DropReason::ForwardedNotFromPeer);
                    debug!(
                        "Rejecting FORWARDED_NPDU from non-BDT sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    return;
                }

                hand_up(
                    ctx,
                    npdu,
                    source_mac,
                    true,
                    TransportProvenance::unverified_bip(
                        BvlcFunction::FORWARDED_NPDU.to_raw(),
                        sender,
                        Some((orig_ip, orig_port)),
                    ),
                    "forwarded",
                );

                // Forward to FDT entries (BDT peers don't need it — they got it directly)
                let (fdt_targets, dedup_count) = {
                    let mut state = bbmd::lock(bbmd);
                    let before = state.fdt_counters().destinations_deduplicated;
                    let targets = state.fdt_forwarding_targets(orig_ip, orig_port);
                    let after = state.fdt_counters().destinations_deduplicated;
                    (targets, after.saturating_sub(before))
                };
                // Annex J.4.5: one that came by broadcast already reached the
                // local subnet, whatever the sender's BDT mask says. Never
                // rebroadcasting it also ends any loop through a BDT row that
                // is this BBMD under another address.
                if needs_local_broadcast && delivery == Delivery::Broadcast {
                    debug!("Forwarded-NPDU arrived by broadcast; skipping local rebroadcast");
                } else if needs_local_broadcast {
                    let local_dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                    let _ = send_forwarded_npdu(ctx, local_dest, orig_ip, orig_port, npdu).await;
                }
                if let Some(fanout) = &ctx.fanout {
                    let addrs: Vec<SocketAddrV4> = fdt_targets
                        .into_iter()
                        .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
                        .collect();
                    fanout.dispatch_forwarded_npdu(orig_ip, orig_port, npdu, addrs, dedup_count);
                } else {
                    let _ = forward_npdu(ctx, npdu, orig_ip, orig_port, &fdt_targets).await;
                }
            } else {
                hand_up(
                    ctx,
                    npdu,
                    source_mac,
                    true,
                    TransportProvenance::unverified_bip(
                        BvlcFunction::FORWARDED_NPDU.to_raw(),
                        sender,
                        Some((orig_ip, orig_port)),
                    ),
                    "forwarded",
                );
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

    // If BBMD, verify sender is a registered foreign device
    if !bbmd::lock(bbmd).is_registered_foreign_device(sender.0, sender.1) {
        debug!(
            "Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender {:?}:{}",
            Ipv4Addr::from(sender.0),
            sender.1
        );
        send_bvlc_result(&ctx.socket, sender, nak).await;
        return;
    }

    hand_up(
        ctx,
        npdu,
        source_mac,
        true,
        TransportProvenance::unverified_bip(
            BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK.to_raw(),
            sender,
            None,
        ),
        "distributed broadcast",
    );

    let (targets, dedup_count) = {
        let mut state = bbmd::lock(bbmd);
        let before = state.fdt_counters().destinations_deduplicated;
        let targets = state.forwarding_targets(sender.0, sender.1);
        let after = state.fdt_counters().destinations_deduplicated;
        (targets, after.saturating_sub(before))
    };

    let local_dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
    let local_ok = if should_force_dbtn_forward_failure(ctx) {
        warn!("Forced DBTN forwarding failure");
        false
    } else {
        send_forwarded_npdu(ctx, local_dest, sender.0, sender.1, npdu).await
    };

    let remote_ok = if let Some(fanout) = &ctx.fanout {
        let addrs: Vec<SocketAddrV4> = targets
            .into_iter()
            .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
            .collect();
        fanout.dispatch_forwarded_npdu(sender.0, sender.1, npdu, addrs, dedup_count)
    } else {
        forward_npdu(ctx, npdu, sender.0, sender.1, &targets).await
    };

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

/// Encode a Forwarded-NPDU carrying `npdu` from `orig_ip:orig_port`.
fn forwarded_npdu(orig_ip: [u8; 4], orig_port: u16, npdu: &[u8]) -> Option<BytesMut> {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    let message = Bvll::ForwardedNpdu {
        origin: SocketAddrV4::new(Ipv4Addr::from(orig_ip), orig_port),
        npdu,
    };
    match encode_bvll(&mut buf, &message) {
        Ok(()) => Some(buf),
        Err(e) => {
            warn!(error = %e, "Failed to encode Forwarded-NPDU");
            None
        }
    }
}

/// Send `npdu` from `orig_ip:orig_port` to `dest` as a Forwarded-NPDU. A
/// failed send is logged through the throttle in
/// [`ReceiveDrops::forward_failed`].
async fn send_forwarded_npdu(
    ctx: &RecvContext,
    dest: SocketAddrV4,
    orig_ip: [u8; 4],
    orig_port: u16,
    npdu: &[u8],
) -> bool {
    let Some(buf) = forwarded_npdu(orig_ip, orig_port, npdu) else {
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
/// when there are many FDT entries (up to 512).
async fn forward_npdu(
    ctx: &RecvContext,
    npdu: &[u8],
    orig_ip: [u8; 4],
    orig_port: u16,
    targets: &[([u8; 4], u16)],
) -> bool {
    if targets.is_empty() {
        return true;
    }
    let Some(buf) = forwarded_npdu(orig_ip, orig_port, npdu) else {
        return false;
    };
    let frame = buf.freeze();
    let mut all_sent = true;

    for (i, &(ip, port)) in targets.iter().enumerate() {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
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
