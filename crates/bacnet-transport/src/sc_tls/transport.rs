//! Shared construction of a reconnecting BACnet/SC node transport.

use std::time::Duration;

use bacnet_types::error::Error;

use super::{ScNodeTlsConfig, TlsWebSocket};
use crate::sc::{ScReconnectConfig, ScTransport};
use crate::sc_frame::{Vmac, BROADCAST_VMAC, UNKNOWN_VMAC};

/// Complete configuration for one TLS hub connection and its reconnect paths.
#[derive(Clone)]
pub struct ScTlsTransportConfig {
    /// Primary Hub Connection URI.
    pub primary_hub_url: String,
    /// Optional Failover Hub Connection URI.
    pub failover_hub_url: Option<String>,
    /// Stable local virtual MAC address.
    pub local_vmac: Vmac,
    /// Stable, caller-provisioned device UUID.
    pub device_uuid: [u8; 16],
    /// Interval between node-originated heartbeat probes.
    pub heartbeat_interval_ms: u64,
    /// Disconnect threshold while a heartbeat acknowledgement is outstanding.
    pub heartbeat_timeout_ms: u64,
    /// Optional reconnect policy.
    pub reconnect: Option<ScReconnectConfig>,
    /// Maximum received NPDU length advertised during Connect-Request.
    pub max_npdu_length: u16,
    /// Maximum received BVLC message length advertised during Connect-Request.
    pub max_bvlc_length: u16,
    /// Optional outer bound for only the initial primary TCP/TLS/WebSocket dial.
    pub initial_dial_timeout: Option<Duration>,
}

impl ScTlsTransportConfig {
    /// Configure the required hub identity and local persistent identity.
    pub fn new(
        primary_hub_url: impl Into<String>,
        local_vmac: Vmac,
        device_uuid: [u8; 16],
    ) -> Self {
        Self {
            primary_hub_url: primary_hub_url.into(),
            failover_hub_url: None,
            local_vmac,
            device_uuid,
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            reconnect: None,
            max_npdu_length: crate::sc_limits::LOCAL_RECEIVE_NPDU_CAPACITY,
            max_bvlc_length: crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH,
            initial_dial_timeout: None,
        }
    }

    /// Set or clear the Failover Hub Connection URI.
    pub fn with_failover_hub(mut self, url: Option<impl Into<String>>) -> Self {
        self.failover_hub_url = url.map(Into::into);
        self
    }

    /// Set the heartbeat interval and outstanding-ack timeout.
    pub fn with_heartbeat(mut self, interval_ms: u64, timeout_ms: u64) -> Self {
        self.heartbeat_interval_ms = interval_ms;
        self.heartbeat_timeout_ms = timeout_ms;
        self
    }

    /// Set or clear the reconnect policy.
    pub fn with_reconnect(mut self, reconnect: Option<ScReconnectConfig>) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Set the advertised and enforced router-capable receive limits.
    pub fn with_receive_limits(mut self, max_npdu_length: u16, max_bvlc_length: u16) -> Self {
        self.max_npdu_length = max_npdu_length;
        self.max_bvlc_length = max_bvlc_length;
        self
    }

    /// Set or clear the initial primary-dial timeout.
    pub fn with_initial_dial_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.initial_dial_timeout = timeout;
        self
    }

    /// Validate local configuration before TLS or socket I/O.
    pub fn validate(&self) -> Result<(), Error> {
        if self.primary_hub_url.trim().is_empty() {
            return Err(Error::Encoding(
                "SC primary hub URL must not be empty".into(),
            ));
        }
        if self.local_vmac == UNKNOWN_VMAC || self.local_vmac == BROADCAST_VMAC {
            return Err(Error::Encoding(
                "SC local VMAC must not be UNKNOWN or BROADCAST".into(),
            ));
        }
        if self.device_uuid == [0; 16] {
            return Err(Error::Encoding(
                "SC device UUID must not be all zero".into(),
            ));
        }
        if !(3_000..=300_000).contains(&self.heartbeat_interval_ms)
            || self.heartbeat_timeout_ms <= self.heartbeat_interval_ms
        {
            return Err(Error::Encoding(
                "SC heartbeat interval must be 3000..=300000 ms and timeout must be greater".into(),
            ));
        }
        if let Some(reconnect) = &self.reconnect {
            reconnect.validate()?;
        }
        crate::sc_limits::validate_router_receive_limits(self.max_npdu_length, self.max_bvlc_length)
    }
}

/// Dial the primary hub and construct one consistently configured SC transport.
///
/// If the primary dial fails while a failover URL is configured, the failure is
/// retained in an unavailable initial socket. Normal `ScTransport` startup then
/// consumes that error and follows its existing failover handshake semantics.
pub async fn connect_sc_transport(
    config: ScTlsTransportConfig,
    tls: ScNodeTlsConfig,
) -> Result<ScTransport<TlsWebSocket>, Error> {
    config.validate()?;
    let primary = if let Some(timeout) = config.initial_dial_timeout {
        tokio::time::timeout(
            timeout,
            TlsWebSocket::connect(&config.primary_hub_url, tls.clone()),
        )
        .await
        .unwrap_or_else(|_| Err(Error::Timeout(timeout)))
    } else {
        TlsWebSocket::connect(&config.primary_hub_url, tls.clone()).await
    };
    let ws = match primary {
        Ok(ws) => ws,
        Err(error) if config.failover_hub_url.is_some() => TlsWebSocket::from_initial_error(error),
        Err(error) => return Err(error),
    };

    let primary_url = config.primary_hub_url.clone();
    let primary_tls = tls.clone();
    let mut transport = ScTransport::new(ws, config.local_vmac)
        .with_device_uuid(config.device_uuid)
        .with_hub_urls(&config.primary_hub_url, config.failover_hub_url.as_deref())
        .with_max_npdu_length(config.max_npdu_length)
        .with_max_bvlc_length(config.max_bvlc_length)
        .with_heartbeat_interval_ms(config.heartbeat_interval_ms)
        .with_heartbeat_timeout_ms(config.heartbeat_timeout_ms)
        .with_connector(move || {
            let url = primary_url.clone();
            let tls = primary_tls.clone();
            async move { TlsWebSocket::connect(&url, tls).await }
        });
    if let Some(reconnect) = config.reconnect {
        transport = transport.with_reconnect(reconnect);
    }
    if let Some(failover_url) = config.failover_hub_url {
        transport = transport.with_failover_connector(move || {
            let url = failover_url.clone();
            let tls = tls.clone();
            async move { TlsWebSocket::connect(&url, tls).await }
        });
    }
    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ScTlsTransportConfig {
        ScTlsTransportConfig::new("wss://hub.example", [1; 6], [2; 16])
    }

    #[test]
    fn defaults_are_router_capable() {
        let config = config();
        assert_eq!(
            (config.max_npdu_length, config.max_bvlc_length),
            (1497, 5705)
        );
        config.validate().unwrap();
    }

    #[test]
    fn validation_precedes_dial_for_identity_timing_reconnect_and_limits() {
        let cases = [
            config().with_receive_limits(1496, 5705),
            config().with_receive_limits(1497, 5704),
            config().with_heartbeat(2999, 60_000),
            ScTlsTransportConfig::new("wss://hub.example", [0; 6], [2; 16]),
            ScTlsTransportConfig::new("wss://hub.example", [1; 6], [0; 16]),
            config().with_reconnect(Some(ScReconnectConfig {
                initial_delay_ms: 10,
                max_delay_ms: 9,
                max_retries: 1,
                retry_forever: false,
            })),
        ];
        for invalid in cases {
            assert!(invalid.validate().is_err());
        }
    }
}
