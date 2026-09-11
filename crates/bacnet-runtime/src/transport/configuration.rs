use super::*;

#[cfg(feature = "sc")]
pub(super) fn select_initial_sc_websocket<W>(
    primary: Result<W, bacnet_types::error::Error>,
    has_failover: bool,
) -> Result<InitialScWebSocket<W>, bacnet_types::error::Error> {
    match primary {
        Ok(ws) => Ok(InitialScWebSocket::Connected(ws)),
        Err(error) if has_failover => Ok(InitialScWebSocket::Unavailable(error.to_string().into())),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "sc")]
fn build_sc_tls_config(
    ca_path: Option<&str>,
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<ScNodeTlsConfig, bacnet_types::error::Error> {
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

    fn required<'a>(
        value: Option<&'a str>,
        label: &str,
    ) -> Result<&'a str, bacnet_types::error::Error> {
        value.filter(|path| !path.is_empty()).ok_or_else(|| {
            bacnet_types::error::Error::Encoding(format!(
                "{label} must be a nonempty path for BACnet/SC mutual TLS"
            ))
        })
    }
    let ca_path = required(ca_path, "ca_cert")?;
    let cert_path = required(cert_path, "client_cert")?;
    let key_path = required(key_path, "client_key")?;

    let ca_data = std::fs::read(ca_path).map_err(|error| {
        bacnet_types::error::Error::Encoding(format!("failed to read SC CA certificate: {error}"))
    })?;
    let ca_certs = CertificateDer::pem_slice_iter(&ca_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            bacnet_types::error::Error::Encoding(format!(
                "failed to parse SC CA certificate: {error}"
            ))
        })?;
    let cert_data = std::fs::read(cert_path).map_err(|error| {
        bacnet_types::error::Error::Encoding(format!(
            "failed to read SC client certificate: {error}"
        ))
    })?;
    let certs = CertificateDer::pem_slice_iter(&cert_data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            bacnet_types::error::Error::Encoding(format!(
                "failed to parse SC client certificate: {error}"
            ))
        })?;
    let key_data = std::fs::read(key_path).map_err(|error| {
        bacnet_types::error::Error::Encoding(format!("failed to read SC client key: {error}"))
    })?;
    let key = PrivateKeyDer::from_pem_slice(&key_data).map_err(|error| {
        bacnet_types::error::Error::Encoding(format!("failed to parse SC client key: {error}"))
    })?;
    ScNodeTlsConfig::from_der(ca_certs, certs, key)
}

impl RuntimeTransport {
    pub(crate) fn validate_config(config: &AttachmentConfig) -> Result<(), RuntimeError> {
        match &config.transport {
            TransportConfig::Bip(bip) => {
                bip.interface.parse::<Ipv4Addr>().map_err(|error| {
                    RuntimeError::invalid_attachment_config(
                        config.id,
                        format!(
                            "attachment {} has invalid B/IP interface {}: {error}",
                            config.id, bip.interface
                        ),
                    )
                })?;
                bip.broadcast
                    .as_deref()
                    .unwrap_or("255.255.255.255")
                    .parse::<Ipv4Addr>()
                    .map_err(|error| {
                        RuntimeError::invalid_attachment_config(
                            config.id,
                            format!(
                                "attachment {} has invalid B/IP broadcast: {error}",
                                config.id
                            ),
                        )
                    })?;
                match (&bip.bbmd_address, bip.foreign_device_ttl) {
                    (None, None) => {}
                    (Some(address), Some(ttl)) => {
                        address.parse::<SocketAddrV4>().map_err(|error| {
                            RuntimeError::invalid_attachment_config(
                                config.id,
                                format!(
                                    "attachment {} has invalid B/IP BBMD address {address}: {error}",
                                    config.id
                                ),
                            )
                        })?;
                        if ttl == 0 {
                            return Err(RuntimeError::invalid_attachment_config(
                                config.id,
                                "B/IP foreign-device TTL must be non-zero",
                            ));
                        }
                    }
                    _ => {
                        return Err(RuntimeError::invalid_attachment_config(
                            config.id,
                            "B/IP BBMD address and foreign-device TTL must both be provided or omitted",
                        ));
                    }
                }
            }
            TransportConfig::Mstp(mstp) => {
                if mstp.device.trim().is_empty() {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "MS/TP serial device must not be empty",
                    ));
                }
                if !matches!(mstp.baud, 9_600 | 19_200 | 38_400 | 76_800) {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "MS/TP baud must be one of 9600, 19200, 38400, or 76800",
                    ));
                }
                if mstp.mac > 127 || mstp.max_master > 127 || mstp.mac > mstp.max_master {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "MS/TP mac and max_master must be in 0..=127 with mac <= max_master",
                    ));
                }
                if mstp.max_info_frames == 0 {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "MS/TP max_info_frames must be non-zero",
                    ));
                }
            }
            TransportConfig::Sc(sc) => {
                if !sc.primary_hub.starts_with("wss://") {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC primary hub must use wss://",
                    ));
                }
                if sc.failover_hubs.len() > 1
                    || sc
                        .failover_hubs
                        .iter()
                        .any(|hub| !hub.starts_with("wss://"))
                {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC accepts at most one wss:// failover hub",
                    ));
                }
                if sc.local_vmac == [0; 6] || sc.local_vmac == [0xff; 6] {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC local VMAC must not be all-zero or broadcast",
                    ));
                }
                if [
                    sc.ca_cert.as_deref(),
                    sc.client_cert.as_deref(),
                    sc.client_key.as_deref(),
                ]
                .into_iter()
                .any(|path| path.is_none_or(str::is_empty))
                {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC requires nonempty CA certificate, client certificate, and client key paths",
                    ));
                }
                if !(3_000..=300_000).contains(&sc.heartbeat_interval_ms)
                    || sc.heartbeat_timeout_ms <= sc.heartbeat_interval_ms
                {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC heartbeat interval must be 3000..=300000 ms and timeout must be greater",
                    ));
                }
                if sc.reconnect_initial_delay_ms == 0
                    || sc.reconnect_max_delay_ms < sc.reconnect_initial_delay_ms
                    || sc.reconnect_max_retries == 0
                {
                    return Err(RuntimeError::invalid_attachment_config(
                        config.id,
                        "BACnet/SC reconnect delays and retries must be non-zero with initial <= maximum",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn start(
        config: &AttachmentConfig,
        cov_channel_capacity: usize,
    ) -> Result<Self, RuntimeError> {
        Self::validate_config(config)?;
        match &config.transport {
            TransportConfig::Bip(bip) => {
                let interface = bip.interface.parse::<Ipv4Addr>().map_err(|error| {
                    RuntimeError::invalid_attachment_config(
                        config.id,
                        format!(
                            "attachment {} has invalid B/IP interface {}: {error}",
                            config.id, bip.interface
                        ),
                    )
                })?;
                let broadcast = bip
                    .broadcast
                    .as_deref()
                    .unwrap_or("255.255.255.255")
                    .parse::<Ipv4Addr>()
                    .map_err(|error| {
                        RuntimeError::invalid_attachment_config(
                            config.id,
                            format!(
                                "attachment {} has invalid B/IP broadcast: {error}",
                                config.id
                            ),
                        )
                    })?;
                let client = if let (Some(address), Some(ttl)) =
                    (&bip.bbmd_address, bip.foreign_device_ttl)
                {
                    let address = address.parse::<SocketAddrV4>().map_err(|error| {
                        RuntimeError::invalid_attachment_config(
                            config.id,
                            format!(
                                "attachment {} has invalid B/IP BBMD address {address}: {error}",
                                config.id
                            ),
                        )
                    })?;
                    let mut transport = BipTransport::new(interface, bip.port, broadcast);
                    transport.register_as_foreign_device(ForeignDeviceConfig {
                        bbmd_ip: *address.ip(),
                        bbmd_port: address.port(),
                        ttl,
                    });
                    BACnetClient::generic_builder()
                        .transport(transport)
                        .cov_channel_capacity(cov_channel_capacity)
                        .build()
                        .await
                } else {
                    BACnetClient::bip_builder()
                        .interface(interface)
                        .port(bip.port)
                        .broadcast_address(broadcast)
                        .cov_channel_capacity(cov_channel_capacity)
                        .build()
                        .await
                }
                .map_err(|error| RuntimeError::attachment_start(config.id, error))?;
                #[cfg(test)]
                run_bip_start_cov_hook(config.id, &client).await;
                Ok(Self::Bip(client))
            }
            #[cfg(feature = "mstp")]
            TransportConfig::Mstp(mstp) => {
                let serial = TokioSerialPort::open(&SerialConfig {
                    port_name: mstp.device.clone(),
                    baud_rate: mstp.baud,
                })
                .map_err(|error| RuntimeError::serial_unavailable(config.id, error))?;
                let transport = MstpTransport::new(
                    serial,
                    TransportMstpConfig {
                        this_station: mstp.mac,
                        max_master: mstp.max_master,
                        max_info_frames: mstp.max_info_frames,
                        baud_rate: mstp.baud,
                    },
                );
                let client = BACnetClient::generic_builder()
                    .transport(transport)
                    .cov_channel_capacity(cov_channel_capacity)
                    .build()
                    .await
                    .map_err(|error| RuntimeError::attachment_start(config.id, error))?;
                Ok(Self::Mstp(client))
            }
            #[cfg(not(feature = "mstp"))]
            TransportConfig::Mstp(_) => {
                Err(RuntimeError::unsupported_transport(config.id, "MS/TP"))
            }
            #[cfg(feature = "sc")]
            TransportConfig::Sc(sc) => {
                let tls = build_sc_tls_config(
                    sc.ca_cert.as_deref(),
                    sc.client_cert.as_deref(),
                    sc.client_key.as_deref(),
                )
                .map_err(|error| RuntimeError::tls_config(config.id, error))?;
                let primary_url = sc.primary_hub.clone();
                let ws = select_initial_sc_websocket(
                    TlsWebSocket::connect(&primary_url, tls.clone()).await,
                    !sc.failover_hubs.is_empty(),
                )
                .map_err(|error| RuntimeError::sc_connect(config.id, error))?;
                let reconnect_primary_url = primary_url.clone();
                let reconnect_primary_tls = tls.clone();
                let mut transport = ScTransport::new(ws, sc.local_vmac)
                    .with_device_uuid(*config.id.as_bytes())
                    .with_heartbeat_interval_ms(sc.heartbeat_interval_ms)
                    .with_heartbeat_timeout_ms(sc.heartbeat_timeout_ms)
                    .with_reconnect(ScReconnectConfig {
                        initial_delay_ms: sc.reconnect_initial_delay_ms,
                        max_delay_ms: sc.reconnect_max_delay_ms,
                        max_retries: sc.reconnect_max_retries,
                    })
                    .with_connector(move || {
                        let url = reconnect_primary_url.clone();
                        let tls = reconnect_primary_tls.clone();
                        async move {
                            TlsWebSocket::connect(&url, tls)
                                .await
                                .map(InitialScWebSocket::Connected)
                        }
                    });
                if let Some(failover_url) = sc.failover_hubs.first().cloned() {
                    let failover_tls = tls.clone();
                    transport = transport.with_failover_connector(move || {
                        let url = failover_url.clone();
                        let tls = failover_tls.clone();
                        async move {
                            TlsWebSocket::connect(&url, tls)
                                .await
                                .map(InitialScWebSocket::Connected)
                        }
                    });
                }
                let connection_state = transport.connection_state_changes();
                let client = BACnetClient::generic_builder()
                    .transport(transport)
                    .cov_channel_capacity(cov_channel_capacity)
                    .build()
                    .await
                    .map_err(|error| RuntimeError::sc_connect(config.id, error))?;
                Ok(Self::Sc(RuntimeScClient {
                    client,
                    connection_state,
                }))
            }
            #[cfg(not(feature = "sc"))]
            TransportConfig::Sc(_) => {
                Err(RuntimeError::unsupported_transport(config.id, "BACnet/SC"))
            }
        }
    }
}
