//! Sending a confirmed request's response, shared by the general handler and
//! the ReadProperty path (#1713).
use super::*;
use crate::server::audit_reporter::{ReadAuditIntent, WriteAudit};

/// How [`BACnetServer::issue_confirmed_response`] sent a response.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Issued {
    /// A ComplexAck too long for one APDU: started as a segmented send, or
    /// answered with an Abort when segmentation isn't available. Read audits
    /// are not admitted.
    Segmented,
    /// Sent in one APDU, after the read audits were admitted.
    Unsegmented,
}

/// Where a confirmed response goes and the limits it is sent under.
pub(super) struct ResponsePlan<'a, T: TransportPort + 'static> {
    pub(super) services: &'a RequestServices<T>,
    pub(super) request_tasks: &'a super::super::request_tasks::RequestTaskSpawner,
    pub(super) target: ResponseTarget<'a>,
    /// The request's identity, with the effective APDU limit as
    /// `client_max_apdu`.
    pub(super) ack: ComplexAckParams,
    pub(super) segmented_response_available: bool,
}

/// The APDU limit a response to `req` is built and sent under, and whether
/// it may be segmented.
pub(super) fn response_limits(
    config: &ServerConfig,
    route: &bacnet_network::response_route::ResponseRoute,
    req: &bacnet_encoding::apdu::ConfirmedRequest,
    source_network: Option<&NpduAddress>,
) -> (u16, bool) {
    let limit = event_information::limit(
        req.max_apdu_length,
        u32::from(config.device.max_apdu_length()),
    );
    let limit = route
        .max_apdu_length(limit, source_network)
        // Invalid reply authority must not revoke already-admitted service
        // execution. Keep its construction budget; issuance still fails
        // closed on the same invalid route without any fallback.
        .unwrap_or(limit);
    let segmented = req.segmented_response_accepted
        && event_information::can_segment(config.device.segmentation());
    (limit, segmented)
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Send `response`. A ComplexAck longer than the effective APDU limit is
    /// segmented, or draws an Abort when segmentation isn't available.
    /// Otherwise `read_audits` are admitted first, once the response exists
    /// and no database guard is held, then the response is sent.
    pub(super) async fn issue_confirmed_response(
        plan: ResponsePlan<'_, T>,
        response: &Apdu,
        audit: &WriteAudit<'_, T>,
        read_audits: Vec<ReadAuditIntent>,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
        pending: Option<PendingConfirmedRequest>,
    ) -> Issued {
        let ResponsePlan {
            services,
            request_tasks,
            target,
            ack: params,
            segmented_response_available,
        } = plan;
        let RequestServices {
            db,
            network,
            seg_ack_senders,
            seg_send_permits,
            config,
            ..
        } = services;
        let ResponseTarget {
            source_mac,
            source_network,
            route,
        } = target;
        if let Apdu::ComplexAck(ack) = response {
            let mut full_buf = BytesMut::new();
            encode_apdu(&mut full_buf, response).expect("valid APDU encoding");

            if full_buf.len() > params.client_max_apdu as usize {
                // Clause 5.4.5.3 CannotSendSegmentedComplexACK reads both
                // sides of the exchange: case (a), no local capability to
                // transmit segmented messages, and case
                // (b), the client not accepting one. Either way the response
                // fits neither an unsegmented nor a segmented send and draws the
                // same Abort; SendSegmentedComplexACK is available only when
                // the device supports transmitting segments (#381).
                if !segmented_response_available {
                    let abort = Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id: params.invoke_id,
                        abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                    if let Err(e) = Self::issue_terminal_response(
                        network,
                        &buf,
                        source_mac,
                        source_network,
                        route,
                        pending,
                    )
                    .await
                    {
                        warn!(error = %e, "Failed to send Abort for segmentation-not-supported");
                    }
                } else {
                    Self::spawn_segmented_complex_ack(
                        SegmentedSendResources {
                            network,
                            seg_ack_senders,
                            seg_send_permits,
                        },
                        request_tasks,
                        target,
                        params,
                        ack.service_ack.clone(),
                        pending,
                        Duration::from_millis(config.device.apdu_segment_timeout_ms()),
                    );
                }
                return Issued::Segmented;
            }
        }

        // Read execution is observed under the DB read guard, but admission is
        // deferred until the complete response exists and that guard is gone.
        // Segmentation/post-execution transport divergence does not add records.
        audit.admit_reads(db, read_audits).await;
        confirmed_response::send_unsegmented_response(
            network,
            response,
            source_mac,
            source_network,
            route,
            reply_tx,
            pending,
        )
        .await;
        Issued::Unsegmented
    }
}
