use super::*;

pub(super) fn snapshot_to_py(snapshot: bacnet_runtime::ScanSnapshot) -> PyScanSnapshot {
    let (path_kind, path_mac, routed_dnet, routed_dadr) = match snapshot.path {
        DevicePath::Direct { mac } => ("direct".to_owned(), mac, None, None),
        DevicePath::Routed {
            ingress_mac,
            dnet,
            dadr,
        } => ("routed".to_owned(), ingress_mac, Some(dnet), Some(dadr)),
    };
    PyScanSnapshot {
        generation: snapshot.generation,
        attachment_id: attachment_u128(snapshot.device.attachment_id),
        device_instance: snapshot.device.device_instance,
        path_kind,
        path_mac,
        routed_dnet,
        routed_dadr,
        outcomes: snapshot
            .outcomes
            .into_iter()
            .map(|outcome| {
                let value = outcome
                    .raw_value
                    .as_deref()
                    .and_then(|raw| crate::types::decode_raw_value(raw, "").ok());
                let (error_code, retryable, bacnet_class, bacnet_code) = outcome
                    .error
                    .map(|error| {
                        (
                            Some(format!("{:?}", error.code)),
                            error.retryable,
                            error.bacnet_class,
                            error.bacnet_code,
                        )
                    })
                    .unwrap_or((None, false, None, None));
                PyScanOutcome {
                    input_index: outcome.input_index,
                    object_type: outcome.read.object_type,
                    object_instance: outcome.read.object_instance,
                    property_id: outcome.read.property_id,
                    array_index: outcome.read.array_index,
                    value_category: outcome.read.value_category,
                    value,
                    raw_value: outcome.raw_value,
                    error_code,
                    retryable,
                    bacnet_class,
                    bacnet_code,
                }
            })
            .collect(),
        rpm_attempts: snapshot.rpm_attempts,
        rp_fallbacks: snapshot.rp_fallbacks,
        elapsed_ms: snapshot.elapsed.as_millis().try_into().unwrap_or(u64::MAX),
    }
}

pub(super) fn observation_to_py(
    observation: bacnet_runtime::DeviceObservation,
    selected: bool,
) -> PyRuntimeDiscoveredDevice {
    let (path_kind, path_mac, routed_dnet, routed_dadr) = path_to_py(observation.path);
    PyRuntimeDiscoveredDevice {
        attachment_id: attachment_u128(observation.key.attachment_id),
        device_instance: observation.key.device_instance,
        selected,
        path_kind,
        path_mac,
        routed_dnet,
        routed_dadr,
        vendor_id: observation.vendor_id,
        max_apdu_length: observation.max_apdu_length,
        revision: observation.revision,
    }
}

pub(super) fn router_to_py(router: bacnet_runtime::RouterObservation) -> PyRuntimeDiscoveredRouter {
    let (path_kind, path_mac, routed_dnet, routed_dadr) = path_to_py(router.path);
    PyRuntimeDiscoveredRouter {
        attachment_id: attachment_u128(router.attachment_id),
        path_kind,
        path_mac,
        routed_dnet,
        routed_dadr,
        networks: router.networks,
    }
}

pub(super) fn topology_attachment_to_py(
    attachment: bacnet_runtime::AttachmentTopology,
) -> PyRuntimeAttachmentTopology {
    PyRuntimeAttachmentTopology {
        attachment_id: attachment_u128(attachment.attachment_id),
        local_mac: attachment.local_mac,
        bbmds: attachment
            .bbmds
            .into_iter()
            .map(|bbmd| PyRuntimeBbmdSnapshot {
                mac: bbmd.mac,
                bdt: bbmd
                    .bdt
                    .into_iter()
                    .map(|entry| PyRuntimeBdtEntry {
                        ip: entry.ip.to_vec(),
                        port: entry.port,
                        broadcast_mask: entry.broadcast_mask.to_vec(),
                    })
                    .collect(),
                fdt: bbmd
                    .fdt
                    .into_iter()
                    .map(|entry| PyRuntimeFdtEntry {
                        ip: entry.ip.to_vec(),
                        port: entry.port,
                        ttl: entry.ttl,
                        seconds_remaining: entry.seconds_remaining,
                    })
                    .collect(),
            })
            .collect(),
        routers: attachment.routers.into_iter().map(router_to_py).collect(),
        truncated: attachment.truncated,
    }
}

pub(super) fn path_to_py(path: DevicePath) -> (String, Vec<u8>, Option<u16>, Option<Vec<u8>>) {
    match path {
        DevicePath::Direct { mac } => ("direct".to_owned(), mac, None, None),
        DevicePath::Routed {
            ingress_mac,
            dnet,
            dadr,
        } => ("routed".to_owned(), ingress_mac, Some(dnet), Some(dadr)),
    }
}

pub(super) fn event_to_py(event: bacnet_runtime::RuntimeEvent) -> PyRuntimeEvent {
    let mut result = PyRuntimeEvent {
        generation: event.generation,
        sequence: event.sequence,
        attachment_id: event.attachment_id.map(attachment_u128),
        kind: String::new(),
        device_instance: None,
        completed: None,
        total: None,
        final_update: None,
        lost_count: None,
        error_code: None,
        observation_process_id: None,
        subscriber_process_identifier: None,
        initiating_device_identifier: None,
        monitored_object_identifier: None,
        time_remaining: None,
        delivery: None,
        object_identifier: None,
        max_apdu_length: None,
        segmentation_supported: None,
        vendor_id: None,
        udp_source_ip: None,
        udp_source_port: None,
        source_mac: None,
        source_network: None,
        source_address: None,
        bvlc_function: None,
        forwarded_from_ip: None,
        forwarded_from_port: None,
        timestamp: None,
        values: Vec::new(),
    };
    match event.kind {
        bacnet_runtime::EventKind::RuntimeStarted => result.kind = "runtime_started".to_owned(),
        bacnet_runtime::EventKind::RuntimeStopped => result.kind = "runtime_stopped".to_owned(),
        bacnet_runtime::EventKind::AttachmentStateChanged => {
            result.kind = "attachment_state_changed".to_owned();
        }
        bacnet_runtime::EventKind::IAmObservation { observation } => {
            result.kind = "i_am_observation".to_owned();
            result.device_instance = Some(observation.device_instance);
            result.object_identifier =
                Some(u32::from_be_bytes(observation.object_identifier.encode()));
            result.max_apdu_length = Some(observation.max_apdu_length);
            result.segmentation_supported = Some(observation.segmentation_supported.to_raw());
            result.vendor_id = Some(observation.vendor_id);
            result.udp_source_ip = observation.udp_source_ip.map(Vec::from);
            result.udp_source_port = observation.udp_source_port;
            result.source_mac = Some(observation.source_mac.to_vec());
            result.source_network = observation.source_network;
            result.source_address = observation.source_address.map(|address| address.to_vec());
            result.bvlc_function = observation.bvlc_function;
            result.forwarded_from_ip = observation.forwarded_from_ip.map(Vec::from);
            result.forwarded_from_port = observation.forwarded_from_port;
            result.timestamp = Python::attach(|py| {
                py.import("time")?
                    .call_method0("monotonic")?
                    .extract::<f64>()
            })
            .ok()
            .map(|now| now - observation.timestamp.elapsed().as_secs_f64());
        }
        bacnet_runtime::EventKind::UnsolicitedCovNotification { notification } => {
            result.kind = "unsolicited_cov_notification".to_owned();
            result.device_instance =
                Some(notification.initiating_device_identifier.instance_number());
            result.observation_process_id = Some(notification.subscriber_process_identifier);
            result.subscriber_process_identifier = Some(notification.subscriber_process_identifier);
            result.initiating_device_identifier = Some(PyObjectIdentifier::from_rust(
                notification.initiating_device_identifier,
            ));
            result.monitored_object_identifier = Some(PyObjectIdentifier::from_rust(
                notification.monitored_object_identifier,
            ));
            result.time_remaining = Some(notification.time_remaining);
            result.delivery = Some(
                match notification.delivery {
                    bacnet_client::client::COVNotificationDelivery::Confirmed => "confirmed",
                    bacnet_client::client::COVNotificationDelivery::Unconfirmed => "unconfirmed",
                }
                .to_owned(),
            );
            result.source_mac = Some(notification.source_mac.to_vec());
            result.source_network = notification.source_network;
            result.source_address = notification.source_address.map(|address| address.to_vec());
            result.values = notification
                .values
                .into_iter()
                .map(|value| {
                    let decoded = decode_raw_value(Some(&value.raw_value));
                    PyRuntimeCovValue {
                        property_id: value.property_id,
                        array_index: value.array_index,
                        value: decoded,
                        raw_value: value.raw_value,
                        priority: value.priority,
                    }
                })
                .collect();
        }
        bacnet_runtime::EventKind::EventLagged { count, .. } => {
            result.kind = "event_lagged".to_owned();
            result.lost_count = Some(count);
        }
        bacnet_runtime::EventKind::ScanProgress {
            device_instance,
            completed_batches,
            total_batches,
            final_update,
        } => {
            result.kind = "scan_progress".to_owned();
            result.device_instance = Some(device_instance);
            result.completed = Some(completed_batches);
            result.total = Some(total_batches);
            result.final_update = Some(final_update);
        }
        bacnet_runtime::EventKind::CovNotification {
            key,
            time_remaining,
            values,
        } => {
            result.kind = "cov_notification".to_owned();
            result.device_instance = Some(key.device.device_instance);
            result.observation_process_id = Some(key.subscriber_process_id);
            result.subscriber_process_identifier = Some(key.subscriber_process_id);
            result.time_remaining = Some(time_remaining);
            result.values = values
                .into_iter()
                .map(|value| {
                    let decoded = decode_raw_value(Some(&value.raw_value));
                    PyRuntimeCovValue {
                        property_id: value.property_id,
                        array_index: value.array_index,
                        value: decoded,
                        raw_value: value.raw_value,
                        priority: None,
                    }
                })
                .collect();
        }
        bacnet_runtime::EventKind::CovRenewed { key } => {
            result.kind = "cov_renewed".to_owned();
            result.device_instance = Some(key.device.device_instance);
            result.observation_process_id = Some(key.subscriber_process_id);
        }
        bacnet_runtime::EventKind::CovRenewalFailed { key, code } => {
            result.kind = "cov_renewal_failed".to_owned();
            result.device_instance = Some(key.device.device_instance);
            result.observation_process_id = Some(key.subscriber_process_id);
            result.error_code = Some(format!("{code:?}"));
        }
        bacnet_runtime::EventKind::CovNotificationLagged { skipped } => {
            result.kind = "cov_notification_lagged".to_owned();
            result.lost_count = Some(skipped);
        }
        bacnet_runtime::EventKind::IAmObservationLagged { skipped } => {
            result.kind = "i_am_observation_lagged".to_owned();
            result.lost_count = Some(skipped);
        }
        _ => result.kind = "unknown".to_owned(),
    }
    result
}
