//! Construction and pre-start configuration for [`ScTransport`].

use super::*;

fn effective_max_apdu_length(conn: &ScConnection) -> u16 {
    let bvlc_npdu_budget = conn
        .hub_max_bvlc_length
        .saturating_sub(SC_ENCAPSULATED_NPDU_BASE_HEADER_LEN);
    let effective_npdu = conn.hub_max_apdu_length.min(bvlc_npdu_budget);
    effective_npdu.saturating_sub(LOCAL_NPDU_HEADER_LEN)
}

pub(crate) fn publish_effective_max_apdu_length(store: &AtomicU16, conn: &ScConnection) {
    store.store(effective_max_apdu_length(conn), Ordering::Relaxed);
}

impl<W: WebSocketPort> ScTransport<W> {
    /// Stable local APDU receive capacity, available before opening a connection.
    /// Negotiated outgoing limits do not change this declaration.
    pub const LOCAL_RECEIVE_APDU_CAPACITY: u16 = crate::sc_limits::LOCAL_RECEIVE_APDU_CAPACITY;

    /// Create an unstarted transport with an unconfigured, zero UUID placeholder.
    /// Call [`Self::with_device_uuid`] before [`TransportPort::start`].
    /// The supplied local VMAC must be neither all-zero nor broadcast.
    pub fn new(ws: W, local_vmac: Vmac) -> Self {
        let (state_tx, _) = watch::channel(ScConnectionState::Disconnected);
        let (health_tx, _) = watch::channel(TransportHealth::default());
        Self {
            ws: Some(ws),
            ws_shared: None,
            local_vmac,
            device_uuid: [0u8; 16],
            advertised_uris: Vec::new(),
            direct_intake: advertisement::DirectIntake::default(),
            #[cfg(feature = "sc-tls")]
            direct_listener_shutdown: None,
            connection: None,
            local_max_bvlc_length: crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH,
            local_max_npdu_length: crate::sc_limits::LOCAL_RECEIVE_NPDU_CAPACITY,
            effective_max_apdu_length: Arc::new(AtomicU16::new(DEFAULT_MAX_APDU_LENGTH)),
            state_tx,
            health_tx,
            topology_id: None,
            recv_task: None,
            connect_timeout_ms: 10_000,
            heartbeat_interval_ms: heartbeat::DEFAULT_HEARTBEAT_INTERVAL_MS,
            heartbeat_timeout_ms: heartbeat::DEFAULT_HEARTBEAT_TIMEOUT_MS,
            failover_ws: None,
            primary_connector: None,
            failover_connector: None,
            reconnect_config: None,
            npdu_admission_policy: ScNpduAdmissionPolicy::default(),
            npdu_admission: None,
            restore_disconnect_task: Arc::new(StdMutex::new(None)),
            direct_membership: Arc::new(direct_membership::DirectMembership::default()),
            direct: None,
            #[cfg(test)]
            allow_test_heartbeat_timing: false,
        }
    }

    /// Configure the device UUID before start (builder-style).
    ///
    /// The caller must generate it before deployment and persist the same bytes
    /// across restarts for the device's lifetime (Annex AB.1.5.3). No UUID is
    /// generated here. Startup rejects only the all-zero UUID, not RFC version
    /// or variant bits, and rejects only all-zero/broadcast local VMACs.
    ///
    /// After reconnect and heartbeat validation, [`TransportPort::start`] checks
    /// identity before transport-owned I/O or startup state changes. These errors
    /// retain both sockets; a UUID error can be repaired with this setter and
    /// start retried on the same owned WebSocket. This cannot undo caller-owned
    /// WebSocket creation or external dials (including work creating closures),
    /// and is not generic endpoint rollback or a promise that every field has a
    /// repair setter. The transport gives applications no mutable access to
    /// the live connection afterwards; they read its state through
    /// [`connection_state_changes`](Self::connection_state_changes).
    pub fn with_device_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.device_uuid = uuid;
        self
    }

    /// Set the maximum NPDU length advertised by this router-capable SC port.
    ///
    /// Startup validates the complete NPDU/BVLC pair against Table 6-1 and
    /// the required 4,192-octet routed Data Options capacity.
    pub fn with_max_npdu_length(mut self, length: u16) -> Self {
        self.local_max_npdu_length = length;
        self
    }

    /// Set the maximum BVLC message length advertised by this SC port.
    ///
    /// Startup validates the complete NPDU/BVLC pair before transport I/O.
    pub fn with_max_bvlc_length(mut self, length: u16) -> Self {
        self.local_max_bvlc_length = length;
        self
    }

    /// Set the connect handshake timeout in milliseconds (builder-style).
    pub fn with_connect_timeout_ms(mut self, ms: u64) -> Self {
        self.connect_timeout_ms = ms;
        self
    }

    /// Set the heartbeat send interval in milliseconds (builder-style).
    ///
    /// Production transports validate this at [`TransportPort::start`] against
    /// the Annex AB.6.3 configurable heartbeat timeout range of 3..300 seconds.
    pub fn with_heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat ack timeout in milliseconds (builder-style).
    ///
    /// This disconnect timeout must be greater than the heartbeat interval so
    /// the peer has a chance to send a Heartbeat-ACK.
    pub fn with_heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    #[cfg(test)]
    pub(super) fn with_test_heartbeat_timing_ms(
        mut self,
        interval_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        self.heartbeat_interval_ms = interval_ms;
        self.heartbeat_timeout_ms = timeout_ms;
        self.allow_test_heartbeat_timing = true;
        self
    }

    /// Set a failover WebSocket to try if the primary connection fails (builder-style).
    pub fn with_failover(mut self, ws: W) -> Self {
        self.failover_ws = Some(ws);
        self
    }

    /// Enable reconnection with the given configuration.
    ///
    /// When the BACnet/SC connection drops, the transport will attempt to reconnect
    /// using exponential backoff as configured. Configure [`Self::with_connector`]
    /// for true transport-level recovery from a dead WebSocket/TCP/TLS connection;
    /// otherwise reconnect attempts can only reuse the current WebSocket object,
    /// except after a rejection NAK exhausts its heartbeat budget. That socket
    /// cannot be reused: recovery needs a fresh connector or unused failover
    /// socket under the existing retry policy, or remains disconnected.
    /// The local VMAC is preserved across reconnections.
    ///
    /// [`TransportPort::start`] validates this configuration before any transport
    /// startup state changes or I/O, retaining the socket on validation failure.
    /// It cannot undo a caller-owned dial: call [`ScReconnectConfig::validate`]
    /// before dialing if needed. Generic endpoint startup may change its own
    /// state before calling the transport; this is not transactional rollback.
    pub fn with_reconnect(mut self, config: ScReconnectConfig) -> Self {
        self.reconnect_config = Some(config);
        self
    }

    /// Attach stable hub URLs used to identify this SC topology.
    pub fn with_hub_urls(
        mut self,
        primary_hub_url: impl AsRef<str>,
        failover_hub_url: Option<impl AsRef<str>>,
    ) -> Self {
        fn normalize(value: &str) -> String {
            let value = value.trim().trim_end_matches('/');
            let Some((scheme, remainder)) = value.split_once("://") else {
                return value.to_owned();
            };
            let authority_end = remainder.find('/').unwrap_or(remainder.len());
            let (authority, path) = remainder.split_at(authority_end);
            format!(
                "{}://{}{}",
                scheme.to_ascii_lowercase(),
                authority.to_ascii_lowercase(),
                path
            )
        }
        let primary = normalize(primary_hub_url.as_ref());
        let failover = failover_hub_url.map(|url| normalize(url.as_ref()));
        self.topology_id = Some(match failover {
            Some(url) => format!("{primary}|{url}"),
            None => primary,
        });
        self
    }

    /// Subscribe to BACnet/SC connection state changes.
    ///
    /// The returned watch receiver yields the latest known state immediately and
    /// change notifications for subsequent state updates observed by the
    /// transport. Rapid updates can coalesce under Tokio watch semantics, so use
    /// this as a current-state signal rather than a durable transition log.
    pub fn connection_state_changes(&self) -> watch::Receiver<ScConnectionState> {
        self.state_tx.subscribe()
    }

    /// Subscribe to the current operational SC health snapshot.
    pub fn transport_health_changes(&self) -> watch::Receiver<TransportHealth> {
        self.health_tx.subscribe()
    }
}
