//! The ReadProperty path for an admitted confirmed request (#1713).
//!
//! ReadProperty is the commonest confirmed service, and the general handler's
//! future is sized for every service: several kilobytes, which the allocator
//! serves on its slow path for every request. ReadProperty runs here instead,
//! in a future a fraction of that size. It does what the general handler did
//! for it: a ReadProperty writes nothing, so there are no write effects, COV
//! notifications or command runs to follow its response.
use super::issue::ResponsePlan;
use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Run admitted work with its single response owner: ReadProperty on its
    /// own path, every other service in the general handler, boxed so that
    /// handler's size isn't carried by a ReadProperty.
    pub(in crate::server) async fn run_admitted_confirmed_request(
        services: &RequestServices<T>,
        request_tasks: &super::super::request_tasks::RequestTaskSpawner,
        origin: RequestOrigin<'_>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
        ownership: Option<ConfirmedRequestOwnership>,
    ) {
        if req.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            Self::handle_admitted_read_property(
                services,
                request_tasks,
                origin,
                req,
                reply_tx,
                ownership,
            )
            .await;
        } else {
            Box::pin(Self::handle_admitted_confirmed_request(
                services,
                request_tasks,
                origin,
                req,
                reply_tx,
                ownership,
            ))
            .await;
        }
    }

    /// Execute an admitted ReadProperty and send its response.
    pub(super) async fn handle_admitted_read_property(
        services: &RequestServices<T>,
        request_tasks: &super::super::request_tasks::RequestTaskSpawner,
        origin: RequestOrigin<'_>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
        ownership: Option<ConfirmedRequestOwnership>,
    ) {
        let RequestServices {
            db,
            network,
            notification_transactions,
            device_bindings,
            config,
            ..
        } = services;
        let RequestOrigin {
            mac: source_mac,
            network: source_network,
            route,
        } = origin;
        // Only a LifeSafetyOperation carries replay ownership; dropping one
        // here completes nothing, as the general handler's drop did.
        let pending = match ownership {
            Some(ConfirmedRequestOwnership::Generic(pending)) => Some(pending),
            _ => None,
        };
        let (effective_max_apdu, segmented_response_available) =
            issue::response_limits(config, &route, &req, source_network.as_ref());
        let invoke_id = req.invoke_id;
        let local_network = network.local_network_number().get();
        let audit = super::audit_reporter::WriteAudit::new(
            config,
            network,
            notification_transactions,
            device_bindings,
            super::audit_reporter::RequestSource {
                mac: source_mac,
                network: source_network.as_ref(),
                invoke_id,
                local_network,
            },
        )
        .await;
        let mut read_audits = Vec::new();
        let response = confirmed_response::read_property_response_observed(
            db,
            Some(services.live_tables()),
            confirmed_response::ReadProfile {
                execution: crate::device_view::DeviceExecution::FullServer,
                registered_port: config.registered_network_port,
                work_limit: config.read_property_multiple_budget.max_result_elements,
                identity: Some(&config.device),
            },
            &req,
            |db, oid, req, result| {
                let result = match result {
                    Ok(()) => None,
                    Err(Error::Timeout(_) | Error::Reject { .. } | Error::Abort { .. }) => return,
                    Err(error) => Some(confirmed_response::error_fields(error)),
                };
                read_audits.extend(audit.read_intent(
                    db,
                    oid,
                    req.property_identifier,
                    req.property_array_index,
                    result,
                ));
            },
        )
        .await;
        let plan = ResponsePlan {
            services,
            request_tasks,
            target: ResponseTarget {
                source_mac,
                source_network: source_network.as_ref(),
                route: &route,
            },
            ack: ComplexAckParams {
                invoke_id,
                service_choice: req.service_choice,
                client_max_apdu: effective_max_apdu,
                client_max_segments: req.max_segments,
            },
            segmented_response_available,
        };
        Self::issue_confirmed_response(plan, &response, &audit, read_audits, reply_tx, pending)
            .await;
    }
}
