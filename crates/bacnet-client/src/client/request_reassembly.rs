//! Bounded answering-side reassembly for confirmed COV. A lazy worker owns its
//! timers and processes segments and requester Aborts in arrival order. The
//! dispatch task answers queue overflow inline, after releasing the TSM lock.
use std::sync::Weak;

use bacnet_endpoint_core::segmented_request::{Opening, Receipt, Segment, SegmentedRequests};
use bacnet_types::enums::AbortReason;
use tokio::sync::mpsc::error::TrySendError;

use super::*;

/// Segments and Aborts waiting for the reassembly task. A burst bigger than
/// this, faster than the task takes it, is dropped and counted.
const QUEUE: usize = 64;

/// What the dispatch task hands the reassembly task.
pub(super) enum Handed {
    /// One segment of a confirmed request, with its reply path.
    Segment {
        source_mac: MacAddr,
        source_network: Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    },
    /// A requester's Abort (server bit clear).
    Abort {
        source_mac: MacAddr,
        source_network: Option<NpduAddress>,
        invoke_id: u8,
        provenance: TransportProvenance,
    },
}

/// The reassembly task, as the client's transaction table keeps it: its
/// queue, its handle (dropping this aborts it) and what the queue dropped.
pub(crate) struct RequestReassembly {
    queue: mpsc::Sender<Handed>,
    task: Option<JoinHandle<()>>,
    /// Segments and Aborts a full queue dropped.
    pub(crate) full_drops: u64,
    /// The task's table, for tests to read.
    #[cfg(test)]
    pub(crate) table: Weak<SyncMutex<SegmentedRequests>>,
}

impl RequestReassembly {
    /// A reassembly whose task never takes from `queue`, for a test to
    /// fill it.
    #[cfg(test)]
    pub(super) fn stalled(queue: mpsc::Sender<Handed>) -> Self {
        Self {
            queue,
            task: None,
            full_drops: 0,
            table: Weak::new(),
        }
    }

    /// Abort the task and wait until it is gone, its table with it.
    pub(super) async fn end(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for RequestReassembly {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// What starting the reassembly task and handing it a segment take,
/// borrowed from `dispatch_apdu`.
pub(super) struct HandOff<'a, T: TransportPort + 'static> {
    pub(super) tsm: &'a Arc<SyncMutex<Tsm>>,
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) cov_tx: &'a broadcast::Sender<COVEvent>,
    pub(super) registry: Option<&'a Arc<cov_registry::Registry>>,
    pub(super) policy: &'a ConfirmedCOVNotificationAckPolicy,
    pub(super) limits: ResponseLimits,
}

/// What the reassembly task publishes to and warns through.
struct Outlets {
    cov_tx: broadcast::Sender<COVEvent>,
    registry: Option<Arc<cov_registry::Registry>>,
    policy: ConfirmedCOVNotificationAckPolicy,
    decode_warning: WarnThrottle,
    response_warning: WarnThrottle,
}

/// Now, on tokio's clock, so a paused test steps the timers.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The body of `dispatch_apdu`'s branch for a segment of a confirmed
    /// request, when the client receives segments: hand it to the
    /// reassembly task, starting the task on the first one.
    ///
    /// `#[inline]`, so each crate that compiles `dispatch_apdu` compiles
    /// its own copy, and the task's sends share the send path every request
    /// takes; `#[cold]` keeps it out of `dispatch_apdu`. With
    /// `#[inline(never)]` the program used the copy of a crate further up
    /// that also starts a client, which brought a second copy of the B/IP
    /// unicast send: the optimizer stopped inlining the datagram split into
    /// either, about 28 instructions a request (perf-counts runs 45247178797
    /// and 45254404722, #2646).
    #[cold]
    #[inline]
    pub(super) fn hand_off_segment(
        at: HandOff<'_, T>,
        segment_timeout: Duration,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    ) -> Option<(InboundReply, u8)> {
        let handed = Handed::Segment {
            source_mac: MacAddr::from_slice(source_mac),
            source_network: source_network.clone(),
            reply,
            request,
        };
        let mut tsm = at.tsm.lock();

        let reassembly = tsm
            .request_reassembly
            .get_or_insert_with(|| Box::new(Self::start_reassembly(&at, segment_timeout)));
        match reassembly.queue.try_send(handed) {
            Ok(()) => None,
            Err(TrySendError::Full(Handed::Segment { reply, request, .. })) => {
                reassembly.full_drops = reassembly.full_drops.saturating_add(1);
                Some((reply, request.invoke_id))
            }
            Err(TrySendError::Full(Handed::Abort { .. })) => unreachable!("handed a segment"),
            Err(TrySendError::Closed(_)) => {
                tsm.request_reassembly = None;
                None
            }
        }
    }

    /// The body of `dispatch_apdu`'s branch for an Abort with the server
    /// bit clear: hand it to the reassembly task, if one runs, to end the
    /// transfer it names (Clause 5.4.5.2). It draws nothing back. Inline
    /// and cold for the reason [`Self::hand_off_segment`] gives.
    #[cold]
    #[inline]
    pub(super) fn hand_off_abort(
        tsm: &Arc<SyncMutex<Tsm>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        invoke_id: u8,
        provenance: &TransportProvenance,
    ) {
        let mut tsm = tsm.lock();
        let Some(reassembly) = tsm.request_reassembly.as_mut() else {
            return;
        };
        let handed = Handed::Abort {
            source_mac: MacAddr::from_slice(source_mac),
            source_network: source_network.clone(),
            invoke_id,
            provenance: provenance.clone(),
        };
        if reassembly.queue.try_send(handed).is_err() {
            reassembly.full_drops = reassembly.full_drops.saturating_add(1);
        }
    }

    /// Start the reassembly task.
    fn start_reassembly(at: &HandOff<'_, T>, segment_timeout: Duration) -> RequestReassembly {
        let (queue, received) = mpsc::channel(QUEUE);
        let table = SegmentedRequests::new(segment_timeout)
            .with_max_segments(at.limits.max_reassembly_segments);
        let table = Arc::new(SyncMutex::new(table));
        #[cfg(test)]
        let weak = Arc::downgrade(&table);
        let outlets = Outlets {
            cov_tx: at.cov_tx.clone(),
            registry: at.registry.cloned(),
            policy: Arc::clone(at.policy),
            decode_warning: WarnThrottle::default(),
            response_warning: WarnThrottle::default(),
        };
        let task = tokio::spawn(Self::reassemble(
            Arc::downgrade(at.network),
            received,
            table,
            outlets,
        ));
        RequestReassembly {
            queue,
            task: Some(task),
            full_drops: 0,
            #[cfg(test)]
            table: weak,
        }
    }

    /// The reassembly task: take each segment and Abort in turn, and end
    /// each transfer when its timer is due.
    async fn reassemble(
        network: Weak<NetworkLayer<T>>,
        mut queue: mpsc::Receiver<Handed>,
        table: Arc<SyncMutex<SegmentedRequests>>,
        outlets: Outlets,
    ) {
        loop {
            let deadline = table.lock().next_deadline();
            let handed = match deadline {
                Some(at) => {
                    tokio::select! {
                        biased;
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(at)) => {
                            let stalled = table.lock().expire(now());
                            if let Some(network) = network.upgrade() {
                                for opening in stalled {
                                    Self::abort_opening(&network, opening, AbortReason::OTHER)
                                        .await;
                                }
                            }
                            continue;
                        }
                        handed = queue.recv() => handed,
                    }
                }
                None => queue.recv().await,
            };
            let Some(handed) = handed else {
                return;
            };
            let Some(network) = network.upgrade() else {
                return;
            };
            match handed {
                Handed::Segment {
                    source_mac,
                    source_network,
                    reply,
                    request,
                } => {
                    Self::receive_request_segment(
                        &network,
                        &table,
                        &outlets,
                        &source_mac,
                        &source_network,
                        reply,
                        request,
                    )
                    .await;
                }
                Handed::Abort {
                    source_mac,
                    source_network,
                    invoke_id,
                    provenance,
                } => {
                    debug!(
                        invoke_id,
                        "A requester's Abort ends its request's reassembly"
                    );
                    table
                        .lock()
                        .abort(&source_mac, source_network.as_ref(), invoke_id, provenance);
                }
            }
        }
    }

    /// Take one segment of a confirmed request.
    #[allow(clippy::too_many_arguments)]
    async fn receive_request_segment(
        network: &Arc<NetworkLayer<T>>,
        table: &SyncMutex<SegmentedRequests>,
        outlets: &Outlets,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    ) {
        let invoke_id = request.invoke_id;
        let receipt = table.lock().receive(
            Segment {
                source_mac,
                source_network: source_network.as_ref(),
                route: reply.route(),
                request: &request,
            },
            now(),
        );
        match receipt {
            // On MS/TP the reply sender drops unused, so the link answers
            // with Reply Postponed.
            Receipt::Saved | Receipt::Discarded => {}
            Receipt::Ack(ack) => {
                if ack.negative_ack {
                    warn_throttled!(
                        outlets.response_warning,
                        invoke_id,
                        seq = ack.sequence_number,
                        "Asking again for segments of a confirmed request"
                    );
                }
                Self::send_request_segment_ack(network, source_mac, source_network, reply, ack)
                    .await;
            }
            Receipt::Abort(reason) => {
                warn_throttled!(
                    outlets.response_warning,
                    invoke_id,
                    reason = reason.to_raw(),
                    "Aborting a segmented confirmed request"
                );
                Self::send_server_abort(
                    network,
                    source_mac,
                    source_network,
                    reply,
                    invoke_id,
                    reason,
                )
                .await;
            }
            Receipt::Retired(opening) => {
                warn_throttled!(
                    outlets.response_warning,
                    invoke_id,
                    "Aborting a segmented confirmed request whose segment changed route"
                );
                Self::abort_opening(network, *opening, AbortReason::INVALID_APDU_IN_THIS_STATE)
                    .await;
            }
            Receipt::Complete(done) => {
                Self::send_request_segment_ack(
                    network,
                    source_mac,
                    source_network,
                    reply,
                    done.ack,
                )
                .await;
                let Opening {
                    source_mac,
                    source_network,
                    route,
                    ..
                } = done.opening;
                let request = done.request;
                debug!(
                    invoke_id,
                    service = request.service_choice.to_raw(),
                    bytes = request.service_request.len(),
                    "Reassembled a segmented confirmed request"
                );
                // Its answer goes back the way its first segment came.
                let answer = InboundReply::new(route, None);
                Self::receive_confirmed_cov_notification(
                    network,
                    &outlets.cov_tx,
                    &outlets.policy,
                    outlets.registry.as_deref(),
                    &source_mac,
                    &source_network,
                    answer,
                    request,
                    &outlets.decode_warning,
                )
                .await;
            }
        }
    }

    /// Send a Segment-ACK for a request segment, server bit set: here the
    /// client is the side that answers.
    async fn send_request_segment_ack(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        ack: SegmentAckPdu,
    ) {
        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &Apdu::SegmentAck(ack)) {
            warn!(error = %e, "Failed to encode a request's SegmentAck");
            return;
        }
        if let Err(e) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(error = %e, "Failed to send a request's SegmentAck");
        }
    }

    /// Abort a transfer along the path its first segment came by.
    async fn abort_opening(network: &Arc<NetworkLayer<T>>, opening: Opening, reason: AbortReason) {
        let Opening {
            source_mac,
            source_network,
            route,
            invoke_id,
        } = opening;
        Self::send_server_abort(
            network,
            &source_mac,
            &source_network,
            InboundReply::new(route, None),
            invoke_id,
            reason,
        )
        .await;
    }
}

#[cfg(test)]
#[path = "request_reassembly_tests.rs"]
mod tests;
