use super::super::*;

fn invalid_registration_metadata(message: impl Into<String>) -> PyErr {
    to_py_err(bacnet_types::error::Error::OutOfRange(message.into()))
}

fn configure_cov_increment(object: &mut dyn BACnetObject, cov_increment: f32) -> PyResult<()> {
    object
        .write_property(
            bacnet_types::enums::PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(cov_increment),
            None,
        )
        .map_err(to_py_err)
}

fn configure_state_text(
    object: &mut dyn BACnetObject,
    number_of_states: u32,
    state_text: Option<Vec<String>>,
) -> PyResult<()> {
    let Some(state_text) = state_text else {
        return Ok(());
    };
    if state_text.len() != number_of_states as usize {
        return Err(invalid_registration_metadata(format!(
            "state_text must contain exactly {number_of_states} entries"
        )));
    }
    for (offset, text) in state_text.into_iter().enumerate() {
        object
            .write_property(
                bacnet_types::enums::PropertyIdentifier::STATE_TEXT,
                Some(offset as u32 + 1),
                PropertyValue::CharacterString(text),
                None,
            )
            .map_err(to_py_err)?;
    }
    Ok(())
}

#[pymethods]
impl BACnetServer {
    #[new]
    #[pyo3(signature = (
        device_instance,
        device_name="BACnet Device",
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        transport="bip",
        sc_hub=None,
        sc_vmac=None,
        sc_ca_cert=None,
        sc_client_cert=None,
        sc_client_key=None,
        sc_heartbeat_interval_ms=None,
        sc_heartbeat_timeout_ms=None,
        ipv6_interface=None,
        dcc_password=None,
        reinit_password=None,
        *,
        dcc_policy="deny_all",
        dcc_source_restriction=None,
        dcc_disable_rate_limit=None,
        serial_port=None,
        mstp_baud=38400,
        mstp_mac=1,
        mstp_max_master=127,
        mstp_max_info_frames=1,
        bbmd=false,
        bbmd_bdt=None,
        bbmd_bdt_persist_path=None,
        bbmd_accept_foreign_devices=false,
        bbmd_max_fdt_entries=128,
        bbmd_management_acl=None,
        bbmd_wire_management_enabled=false,
        bvll_policy=None,
        bvll_policy_timeout_ms=50,
        bvll_policy_queue_capacity=32,
        bvll_policy_failure_threshold=3,
        bvll_policy_cooldown_ms=1000,
        reuse_port=false,
        max_confirmed_in_flight=64,
        max_unconfirmed_in_flight=32,
        max_confirmed_in_flight_per_peer=16,
        max_unconfirmed_in_flight_per_peer=8,
        confirmed_recovery_reserve=4,
        max_recovery_in_flight_per_peer=1,
        rpm_max_result_elements=256,
        rpm_max_service_ack_bytes=16384,
        alarm_summary_max_objects=4096,
        alarm_summary_max_service_ack_bytes=16384,
        enrollment_summary_max_objects=4096,
        enrollment_summary_max_service_ack_bytes=16384,
        atomic_read_file_max_requested_stream_octets=16384,
        atomic_read_file_max_requested_records=256,
        atomic_read_file_max_service_ack_bytes=16384,
        atomic_write_file_max_stream_payload_octets=16384,
        atomic_write_file_max_records=256,
        atomic_write_file_max_record_payload_bytes=16384,
        read_range_max_returned_items=256,
        read_range_max_service_ack_bytes=16384,
        event_information_max_objects=4096,
        event_information_max_returned_summaries=256,
        event_information_max_service_ack_bytes=16384,
        vendor_name="Rusty BACnet",
        vendor_identifier=555,
        model_name="rusty-bacnet",
        description="",
        firmware_revision="0.1.0",
        application_software_version="0.1.0",
        sc_device_uuid=None,
        apdu_observer=false,
        apdu_observer_capacity=64
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        device_instance: u32,
        device_name: &str,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        transport: &str,
        sc_hub: Option<String>,
        sc_vmac: Option<Vec<u8>>,
        sc_ca_cert: Option<String>,
        sc_client_cert: Option<String>,
        sc_client_key: Option<String>,
        sc_heartbeat_interval_ms: Option<u64>,
        sc_heartbeat_timeout_ms: Option<u64>,
        ipv6_interface: Option<String>,
        dcc_password: Option<String>,
        reinit_password: Option<String>,
        dcc_policy: &str,
        dcc_source_restriction: Option<Vec<(Option<u16>, Vec<u8>)>>,
        dcc_disable_rate_limit: Option<(u32, u64)>,
        serial_port: Option<String>,
        mstp_baud: u32,
        mstp_mac: u8,
        mstp_max_master: u8,
        mstp_max_info_frames: u8,
        bbmd: bool,
        bbmd_bdt: Option<Vec<(String, u16, String)>>,
        bbmd_bdt_persist_path: Option<String>,
        bbmd_accept_foreign_devices: bool,
        bbmd_max_fdt_entries: usize,
        bbmd_management_acl: Option<Vec<String>>,
        bbmd_wire_management_enabled: bool,
        bvll_policy: Option<Py<PyAny>>,
        bvll_policy_timeout_ms: u64,
        bvll_policy_queue_capacity: usize,
        bvll_policy_failure_threshold: usize,
        bvll_policy_cooldown_ms: u64,
        reuse_port: bool,
        max_confirmed_in_flight: usize,
        max_unconfirmed_in_flight: usize,
        max_confirmed_in_flight_per_peer: usize,
        max_unconfirmed_in_flight_per_peer: usize,
        confirmed_recovery_reserve: usize,
        max_recovery_in_flight_per_peer: usize,
        rpm_max_result_elements: usize,
        rpm_max_service_ack_bytes: usize,
        alarm_summary_max_objects: usize,
        alarm_summary_max_service_ack_bytes: usize,
        enrollment_summary_max_objects: usize,
        enrollment_summary_max_service_ack_bytes: usize,
        atomic_read_file_max_requested_stream_octets: usize,
        atomic_read_file_max_requested_records: usize,
        atomic_read_file_max_service_ack_bytes: usize,
        atomic_write_file_max_stream_payload_octets: usize,
        atomic_write_file_max_records: usize,
        atomic_write_file_max_record_payload_bytes: usize,
        read_range_max_returned_items: usize,
        read_range_max_service_ack_bytes: usize,
        event_information_max_objects: usize,
        event_information_max_returned_summaries: usize,
        event_information_max_service_ack_bytes: usize,
        vendor_name: &str,
        vendor_identifier: u16,
        model_name: &str,
        description: &str,
        firmware_revision: &str,
        application_software_version: &str,
        sc_device_uuid: Option<Vec<u8>>,
        apdu_observer: bool,
        apdu_observer_capacity: usize,
    ) -> PyResult<Self> {
        if bbmd && transport != "bip" {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "bbmd=True requires transport='bip'",
            ));
        }
        if reuse_port && transport != "bip" {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "reuse_port=True requires transport='bip'",
            ));
        }
        if bbmd_wire_management_enabled {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "inbound Write-BDT is intentionally unsupported and always NAKs",
            ));
        }
        if !bbmd
            && (bbmd_bdt.is_some()
                || bbmd_bdt_persist_path.is_some()
                || bbmd_accept_foreign_devices
                || bbmd_max_fdt_entries != 128
                || bbmd_management_acl.is_some()
                || bvll_policy.is_some())
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "BBMD options require bbmd=True",
            ));
        }
        if bbmd_max_fdt_entries == 0
            || bbmd_max_fdt_entries > bacnet_transport::bbmd::BbmdState::MAX_FDT_ENTRIES
        {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "bbmd_max_fdt_entries must be in 1..={}",
                bacnet_transport::bbmd::BbmdState::MAX_FDT_ENTRIES
            )));
        }
        let policy = bvll_policy
            .map(|callable| {
                crate::types::PythonBvllPolicyBridge::new(
                    py,
                    callable,
                    bvll_policy_timeout_ms,
                    bvll_policy_queue_capacity,
                    bvll_policy_failure_threshold,
                    bvll_policy_cooldown_ms,
                )
            })
            .transpose()?;

        let (pending_bbmd_transport, bbmd_transport_config, bbmd_control) = if bbmd {
            let interface = interface.parse::<Ipv4Addr>().map_err(|error| {
                pyo3::exceptions::PyValueError::new_err(format!("invalid BBMD interface: {error}"))
            })?;
            let broadcast = broadcast_address.parse::<Ipv4Addr>().map_err(|error| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid BBMD broadcast address: {error}"
                ))
            })?;
            let initial_bdt = crate::types::parse_bdt_entries(bbmd_bdt.unwrap_or_default())?;
            let management_acl =
                crate::types::parse_management_acl(bbmd_management_acl.unwrap_or_default())?;
            let persist_path = bbmd_bdt_persist_path
                .map(|path| {
                    if path.is_empty() {
                        Err(pyo3::exceptions::PyValueError::new_err(
                            "bbmd_bdt_persist_path must not be empty",
                        ))
                    } else {
                        Ok(std::path::PathBuf::from(path))
                    }
                })
                .transpose()?;
            let config = crate::types::PyBbmdTransportConfig {
                interface,
                port,
                broadcast,
                reuse_port,
                initial_bdt,
                persist_path,
                accept_foreign_devices: bbmd_accept_foreign_devices,
                max_fdt_entries: bbmd_max_fdt_entries,
                management_acl,
                policy,
            };
            let transport = config.transport()?;
            let control = transport
                .bbmd_control()
                .expect("configured BBMD creates control capability");
            let control = crate::types::PyBbmdControl::from_rust(control, config.policy.clone());
            (Some(transport), Some(config), Some(control))
        } else {
            (None, None, None)
        };

        let dcc_policy = match dcc_policy {
            "deny_all" => server::DccPolicy::DenyAll,
            "require_password" => server::DccPolicy::RequirePassword,
            "legacy_permissive" => server::DccPolicy::LegacyPermissive,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "dcc_policy must be 'deny_all', 'require_password', or 'legacy_permissive'",
                ))
            }
        };
        dcc_policy
            .validate(&dcc_password)
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let dcc_source_restriction = dcc_source_restriction
            .map(|entries| {
                let restriction = server::DccSourceRestriction::new(
                    entries
                        .into_iter()
                        .map(|(network, address)| match network {
                            None => server::DccSource::Direct(address),
                            Some(network) => server::DccSource::Routed { network, address },
                        })
                        .collect(),
                )?;
                restriction.validate_policy(dcc_policy)?;
                Ok::<_, bacnet_types::error::Error>(restriction)
            })
            .transpose()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let dcc_disable_rate_limit = dcc_disable_rate_limit
            .map(|(capacity, refill_interval_ms)| {
                let limit = server::DccDisableRateLimit {
                    capacity,
                    refill_interval_ms,
                };
                limit.validate()?;
                Ok::<_, bacnet_types::error::Error>(limit)
            })
            .transpose()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let request_admission_policy = server::RequestAdmissionPolicy {
            max_confirmed_in_flight,
            max_unconfirmed_in_flight,
            max_confirmed_in_flight_per_peer,
            max_unconfirmed_in_flight_per_peer,
            confirmed_recovery_reserve,
            max_recovery_in_flight_per_peer,
        };
        request_admission_policy
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let read_property_multiple_budget = server::ReadPropertyMultipleBudget {
            max_result_elements: rpm_max_result_elements,
            max_service_ack_bytes: rpm_max_service_ack_bytes,
        };
        read_property_multiple_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let get_alarm_summary_budget = server::GetAlarmSummaryBudget {
            max_objects: alarm_summary_max_objects,
            max_service_ack_bytes: alarm_summary_max_service_ack_bytes,
        };
        get_alarm_summary_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let get_enrollment_summary_budget = server::GetEnrollmentSummaryBudget {
            max_objects: enrollment_summary_max_objects,
            max_service_ack_bytes: enrollment_summary_max_service_ack_bytes,
        };
        get_enrollment_summary_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let atomic_read_file_budget = server::AtomicReadFileBudget {
            max_requested_stream_octets: atomic_read_file_max_requested_stream_octets,
            max_requested_records: atomic_read_file_max_requested_records,
            max_service_ack_bytes: atomic_read_file_max_service_ack_bytes,
        };
        atomic_read_file_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let read_range_budget = server::ReadRangeBudget {
            max_returned_items: read_range_max_returned_items,
            max_service_ack_bytes: read_range_max_service_ack_bytes,
        };
        let atomic_write_file_budget = server::AtomicWriteFileBudget {
            max_stream_payload_octets: atomic_write_file_max_stream_payload_octets,
            max_records: atomic_write_file_max_records,
            max_record_payload_bytes: atomic_write_file_max_record_payload_bytes,
        };
        atomic_write_file_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        read_range_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        let get_event_information_budget = server::GetEventInformationBudget {
            max_objects: event_information_max_objects,
            max_returned_summaries: event_information_max_returned_summaries,
            max_service_ack_bytes: event_information_max_service_ack_bytes,
        };
        get_event_information_budget
            .validate()
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        if transport == "sc" {
            crate::tls::required_sc_credentials(
                sc_ca_cert.as_deref(),
                sc_client_cert.as_deref(),
                sc_client_key.as_deref(),
            )
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        }
        let sc_device_uuid = crate::sc_identity::device_uuid(transport, sc_device_uuid)?;
        let apdu_observer = PyApduObserverState::configured(apdu_observer, apdu_observer_capacity)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            apdu_observer,
            device_identity: DeviceIdentityConfig {
                instance: device_instance,
                name: device_name.to_string(),
                description: description.to_string(),
                vendor_name: vendor_name.to_string(),
                vendor_id: vendor_identifier,
                model_name: model_name.to_string(),
                firmware_revision: firmware_revision.to_string(),
                application_software_version: application_software_version.to_string(),
            },
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            reuse_port,
            pending_bbmd_transport: Arc::new(std::sync::Mutex::new(pending_bbmd_transport)),
            bbmd_transport_config,
            bbmd_control: Arc::new(std::sync::Mutex::new(bbmd_control)),
            sc_hub,
            sc_vmac,
            sc_device_uuid,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            ipv6_interface,
            serial_port,
            mstp_baud,
            mstp_mac,
            mstp_max_master,
            mstp_max_info_frames,
            dcc_password,
            dcc_policy,
            dcc_source_restriction,
            dcc_disable_rate_limit,
            reinit_password,
            request_admission_policy,
            read_property_multiple_budget,
            get_alarm_summary_budget,
            get_enrollment_summary_budget,
            atomic_read_file_budget,
            atomic_write_file_budget,
            read_range_budget,
            get_event_information_budget,
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Return the capability-limited local BBMD control handle.
    #[getter]
    fn bbmd_control(&self) -> PyResult<crate::types::PyBbmdControl> {
        self.bbmd_control
            .lock()
            .map_err(|_| PyRuntimeError::new_err("BBMD control lock poisoned"))?
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("server is not configured as a B/IP BBMD"))
    }

    /// Test seam for verifying that fallible startup leaves registrations intact.
    #[doc(hidden)]
    fn _pending_registration_count(&self) -> PyResult<usize> {
        Ok(self.lock_pending()?.len())
    }
}

#[allow(clippy::too_many_arguments)]
fn staging_config(
    present_value: f32,
    min_present_value: f32,
    units: u32,
    priority_for_writing: u8,
    stages: Vec<(f32, Vec<bool>, f32)>,
    target_references: Vec<PyObjectIdentifier>,
    stage_names: Option<Vec<String>>,
) -> StagingConfig {
    StagingConfig {
        present_value,
        min_present_value,
        units,
        priority_for_writing,
        stages: stages
            .into_iter()
            .map(|(limit, values, deadband)| BACnetStageLimitValue {
                limit,
                values,
                deadband,
            })
            .collect(),
        target_references: target_references
            .into_iter()
            .map(|reference| BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: reference.to_rust(),
            })
            .collect(),
        stage_names,
    }
}

#[path = "registration_object_basics.rs"]
mod object_basics;

#[path = "registration_object_catalog.rs"]
mod object_catalog;

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;
