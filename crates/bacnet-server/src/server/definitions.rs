/// Result of a confirmed COV notification from the subscriber's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovAckResult {
    /// SimpleAck received — subscriber accepted the notification.
    Ack,
    /// Error or Reject/Abort received — subscriber rejected the notification.
    Error,
}
/// Legacy server transaction state and learned-router cache.
///
/// The allocation and pending-result methods remain available to existing
/// server internals and tests. Standalone confirmed notification paths use the
/// private endpoint-core adapter instead.
pub struct ServerTsm {
    #[allow(dead_code)]
    next_invoke_id: u8,
    /// Oneshot senders keyed by peer MAC and invoke ID. When a result arrives
    /// from the dispatch loop, we send it directly — no polling needed.
    #[allow(dead_code)]
    pending: HashMap<TsmKey, oneshot::Sender<CovAckResult>>,
    /// Router MACs learned per remote network, Clause 6.5.3 method 4: "using
    /// the local broadcast MAC address in the initial transmission to a device
    /// on a remote DNET and noting the SA associated with any subsequent
    /// responses from the remote device" (#375). Consulted so later confirmed
    /// sends to that DNET can unicast to the router instead of broadcasting.
    routers: HashMap<u16, MacAddr>,
}

/// Cap on learned router entries; a full cache just means later networks keep
/// using the (always-correct) broadcast form of Clause 6.5.3.
const MAX_LEARNED_ROUTERS: usize = 64;

impl ServerTsm {
    fn new() -> Self {
        Self {
            next_invoke_id: 0,
            pending: HashMap::new(),
            routers: HashMap::new(),
        }
    }

    /// Allocate the next invoke ID and register a oneshot channel for the result.
    /// Returns (invoke_id, receiver).
    #[allow(dead_code)]
    fn allocate(&mut self, peer: TsmPeer) -> Option<(u8, oneshot::Receiver<CovAckResult>)> {
        for offset in 0..=u8::MAX {
            let id = self.next_invoke_id.wrapping_add(offset);
            if !self
                .pending
                .contains_key(&(peer.0.clone(), peer.1.clone(), id))
            {
                self.next_invoke_id = id.wrapping_add(1);
                let rx = self.register(peer, id);
                return Some((id, rx));
            }
        }
        None
    }

    /// Register or replace the pending receiver for a peer/invoke-id pair.
    #[allow(dead_code)]
    fn register(&mut self, peer: TsmPeer, invoke_id: u8) -> oneshot::Receiver<CovAckResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.insert((peer.0, peer.1, invoke_id), tx);
        rx
    }

    /// Record a result from the dispatch loop (SimpleAck, Error, etc.).
    /// Sends immediately through the oneshot channel.
    #[allow(dead_code)]
    fn record_result(
        &mut self,
        peer: &MacAddr,
        network: Option<&NpduAddress>,
        invoke_id: u8,
        result: CovAckResult,
    ) -> bool {
        if let Some(tx) = self
            .pending
            .remove(&(peer.clone(), network.cloned(), invoke_id))
        {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    /// Remove a pending entry (cleanup on completion or exhaustion).
    #[allow(dead_code)]
    fn remove(&mut self, peer: &TsmPeer, invoke_id: u8) {
        self.pending
            .remove(&(peer.0.clone(), peer.1.clone(), invoke_id));
    }

    /// Correlate an inbound response with the transaction awaiting it (#375).
    ///
    /// Three key shapes are tried, most specific first:
    /// 1. exactly as the sender registered it — the immediate MAC plus any
    ///    routed identity;
    /// 2. the router-unknown form — an empty local half with the routed
    ///    identity, used when the request went out via the Clause 6.5.3
    ///    broadcast DA and the delivering router's MAC was unknowable at
    ///    registration;
    /// 3. the legacy wildcard `(empty, None)`, which nothing registers today
    ///    but which older callers may still expect.
    ///
    /// A hit that carries a routed identity also teaches the router cache:
    /// the response's immediate MAC is "the SA associated with [a] subsequent
    /// response from the remote device" (Clause 6.5.3 method 4).
    #[allow(dead_code)]
    fn record_result_correlated(
        &mut self,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        result: CovAckResult,
    ) -> bool {
        let hit = self.record_result(source_mac, source_network, invoke_id, result)
            || (source_network.is_some()
                && self.record_result(&MacAddr::new(), source_network, invoke_id, result))
            || self.record_result(&MacAddr::new(), None, invoke_id, result);
        if hit {
            if let Some(address) = source_network {
                self.learn_router(address.network, source_mac);
            }
        }
        hit
    }

    /// Cache `router` as the way to reach `network`, bounded by
    /// [`MAX_LEARNED_ROUTERS`].
    fn learn_router(&mut self, network: u16, router: &MacAddr) {
        if router.is_empty() {
            return;
        }
        if self.routers.len() >= MAX_LEARNED_ROUTERS && !self.routers.contains_key(&network) {
            return;
        }
        self.routers.insert(network, router.clone());
    }

    /// The learned router MAC for `network`, if any.
    fn cached_router(&self, network: u16) -> Option<MacAddr> {
        self.routers.get(&network).cloned()
    }
}

/// Data from a TimeSynchronization request.
#[derive(Debug, Clone)]
pub struct TimeSyncData {
    /// Raw service request bytes (caller can decode if needed).
    pub raw_service_data: Bytes,
    /// Whether this was a UTC time sync (vs. local).
    pub is_utc: bool,
}

/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    /// Transport and network roles active in this server process, used as the
    /// authoritative source for PICS data-link and network-layer claims.
    pub runtime_capabilities: crate::pics::RuntimeCapabilities,
    /// Per-service GetAlarmSummary database scan and encoded response limits.
    pub get_alarm_summary_budget: GetAlarmSummaryBudget,
    /// Local complete-response limits for GetEnrollmentSummary.
    pub get_enrollment_summary_budget: GetEnrollmentSummaryBudget,
    /// Local AtomicReadFile raw-count and complete service-ACK limits.
    pub atomic_read_file_budget: AtomicReadFileBudget,
    /// Local AtomicWriteFile payload admission limits.
    pub atomic_write_file_budget: AtomicWriteFileBudget,
    /// ReadRange directional page item and logical service-byte limits.
    pub read_range_budget: ReadRangeBudget,
    /// GetEventInformation database admission and strict response-page limits.
    pub get_event_information_budget: GetEventInformationBudget,
    /// Per-service RPM work and encoded response limits (finite by default).
    pub read_property_multiple_budget: ReadPropertyMultipleBudget,
    /// Local interface to bind.
    pub interface: Ipv4Addr,
    /// UDP port (default 0xBAC0 = 47808).
    pub port: u16,
    /// Directed broadcast address.
    pub broadcast_address: Ipv4Addr,
    /// Maximum APDU length accepted.
    pub max_apdu_length: u32,
    /// Segmentation support level.
    ///
    /// Enforced, not just advertised: the dispatch loop reassembles inbound
    /// segmented requests only under `BOTH`/`RECEIVE` and transmits
    /// segmented responses only under `BOTH`/`TRANSMIT` (Clauses 5.4.5.1 and
    /// 5.4.5.3); anything else draws a SEGMENTATION_NOT_SUPPORTED Abort. The
    /// default is `NONE`, so a default-configured server refuses segmented
    /// traffic in both directions — set this to what the device should
    /// actually honor.
    pub segmentation_supported: Segmentation,
    /// Vendor identifier.
    pub vendor_id: u16,
    /// Timeout in ms before retrying a failed confirmed COV notification send (default 3000ms).
    pub cov_retry_timeout_ms: u64,
    /// Optional observer invoked after a time-synchronization request is accepted.
    pub on_time_sync: Option<Arc<dyn Fn(TimeSyncData) + Send + Sync>>,
    /// Optional LifeSafetyOperation authorization policy.
    ///
    /// Absence is fail-closed: requests receive SERVICES /
    /// SERVICE_REQUEST_DENIED before object mutation.
    pub life_safety_operation_authorizer: Option<LifeSafetyOperationAuthorizer>,
    /// Exactly one explicitly configured Audit Log notification sink.
    ///
    /// Absence is fail-closed; the server never selects a sink by database
    /// iteration order.
    pub audit_notification_sink: Option<ObjectIdentifier>,
    /// Optional fast, nonblocking ConfirmedAuditNotification authorizer.
    ///
    /// Absence, `false`, or a panic denies the request before mutation.
    pub audit_notification_authorizer: Option<AuditNotificationAuthorizer>,
    /// Optional fast, nonblocking UnconfirmedAuditNotification authorizer.
    ///
    /// Absence, `false`, or a panic silently denies the request before mutation.
    pub unconfirmed_audit_notification_authorizer: Option<UnconfirmedAuditNotificationAuthorizer>,
    /// Optional password required for DeviceCommunicationControl.
    pub dcc_password: Option<String>,
    /// Local DCC authorization. Supplying a password alone does not enable DCC.
    pub dcc_policy: DccPolicy,
    /// Optional exact claimed-source restriction, valid only with RequirePassword.
    pub dcc_source_restriction: Option<DccSourceRestriction>,
    /// Optional global DISABLE_INITIATION budget; does not enable DCC authorization.
    pub dcc_disable_rate_limit: Option<DccDisableRateLimit>,
    /// Optional password required for ReinitializeDevice.
    pub reinit_password: Option<String>,
    /// Enable periodic fault detection / reliability evaluation.
    /// When true, the server invokes every object's opt-in, object-owned
    /// reliability evaluation hook every 10 seconds. Stock objects currently
    /// inherit the no-op default.
    ///
    /// This governs reliability evaluation only. Event Enrollment evaluation
    /// is configured separately via [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub enable_fault_detection: bool,
    /// Enable periodic Event Enrollment evaluation (default `true`).
    ///
    /// When true, the server re-reads the property each Event Enrollment object
    /// names in its `Object_Property_Reference` and applies the configured event
    /// algorithm. Startup: the task is spawned by [`start`](BACnetServer::start)
    /// and its first pass runs immediately, then once per interval. Shutdown:
    /// [`stop`](BACnetServer::stop) aborts it and awaits the abort.
    ///
    /// This switch governs the evaluation task; it is not the per-object
    /// `Event_Detection_Enable` property of ASHRAE 135-2020 Clause 13.2.2.1.
    /// Setting it false stops evaluation without performing the reset that
    /// clause requires of a disabled detector (`Event_State` to NORMAL, with the
    /// corresponding timestamp and acknowledgment state), so a device carrying
    /// active enrollments will hold whatever state it last detected.
    ///
    /// Evaluation is a no-op on databases holding no Event Enrollment objects,
    /// so the default is on.
    ///
    /// Successful enabled transitions commit `Event_State`,
    /// `Acked_Transitions`, and `Event_Time_Stamps` atomically and are then
    /// routed through the shared EventNotification sender. Event Enrollment
    /// message text remains intentionally absent, and exact event-specific
    /// notification values are deferred to the payload projection work.
    pub enable_event_enrollment: bool,
    /// Interval in seconds between Event Enrollment evaluation passes (default 10).
    ///
    /// This is a sampling cadence with no basis in ASHRAE 135-2020, which
    /// prescribes no evaluation frequency and leaves acquisition of a monitored
    /// value a local matter (Clause 12.12). It is not the `Time_Delay` of an
    /// event algorithm, which is how long a condition must persist before a
    /// transition is indicated (Clause 13.3) — a coarse interval delays
    /// detection and can miss a condition that both appears and clears between
    /// two passes.
    ///
    /// A value of `0` is clamped to one second. Ignored when
    /// [`enable_event_enrollment`](Self::enable_event_enrollment) is false.
    pub event_enrollment_interval_secs: u64,
    /// Discovery rate-limiting and duplicate suppression policy.
    pub discovery_policy: DiscoveryPolicy,
    /// COV quota, rate accounting, and notification work budget policy.
    pub cov_policy: CovPolicy,
    /// Positive independent top-level request handler limits.
    pub request_admission_policy: RequestAdmissionPolicy,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("runtime_capabilities", &self.runtime_capabilities)
            .field("get_alarm_summary_budget", &self.get_alarm_summary_budget)
            .field(
                "get_enrollment_summary_budget",
                &self.get_enrollment_summary_budget,
            )
            .field("atomic_read_file_budget", &self.atomic_read_file_budget)
            .field("atomic_write_file_budget", &self.atomic_write_file_budget)
            .field("read_range_budget", &self.read_range_budget)
            .field(
                "get_event_information_budget",
                &self.get_event_information_budget,
            )
            .field(
                "read_property_multiple_budget",
                &self.read_property_multiple_budget,
            )
            .field("interface", &self.interface)
            .field("port", &self.port)
            .field("broadcast_address", &self.broadcast_address)
            .field("max_apdu_length", &self.max_apdu_length)
            .field("segmentation_supported", &self.segmentation_supported)
            .field("vendor_id", &self.vendor_id)
            .field("cov_retry_timeout_ms", &self.cov_retry_timeout_ms)
            .field(
                "on_time_sync",
                &self.on_time_sync.as_ref().map(|_| "<callback>"),
            )
            .field(
                "life_safety_operation_authorizer",
                &self
                    .life_safety_operation_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field("audit_notification_sink", &self.audit_notification_sink)
            .field(
                "audit_notification_authorizer",
                &self
                    .audit_notification_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field(
                "unconfirmed_audit_notification_authorizer",
                &self
                    .unconfirmed_audit_notification_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field("dcc_password", &self.dcc_password.as_ref().map(|_| "***"))
            .field("dcc_policy", &self.dcc_policy)
            .field("dcc_source_restriction", &self.dcc_source_restriction)
            .field("dcc_disable_rate_limit", &self.dcc_disable_rate_limit)
            .field(
                "reinit_password",
                &self.reinit_password.as_ref().map(|_| "***"),
            )
            .field("enable_fault_detection", &self.enable_fault_detection)
            .field("enable_event_enrollment", &self.enable_event_enrollment)
            .field(
                "event_enrollment_interval_secs",
                &self.event_enrollment_interval_secs,
            )
            .field("discovery_policy", &self.discovery_policy)
            .field("cov_policy", &self.cov_policy)
            .field("request_admission_policy", &self.request_admission_policy)
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            runtime_capabilities: crate::pics::RuntimeCapabilities::default(),
            interface: Ipv4Addr::UNSPECIFIED,
            read_property_multiple_budget: ReadPropertyMultipleBudget::default(),
            get_alarm_summary_budget: GetAlarmSummaryBudget::default(),
            get_enrollment_summary_budget: GetEnrollmentSummaryBudget::default(),
            atomic_read_file_budget: AtomicReadFileBudget::default(),
            atomic_write_file_budget: AtomicWriteFileBudget::default(),
            read_range_budget: ReadRangeBudget::default(),
            get_event_information_budget: GetEventInformationBudget::default(),
            port: 0xBAC0,
            broadcast_address: Ipv4Addr::BROADCAST,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 0,
            cov_retry_timeout_ms: 3000,
            on_time_sync: None,
            life_safety_operation_authorizer: None,
            audit_notification_sink: None,
            audit_notification_authorizer: None,
            unconfirmed_audit_notification_authorizer: None,
            dcc_password: None,
            dcc_policy: DccPolicy::default(),
            dcc_source_restriction: None,
            dcc_disable_rate_limit: None,
            reinit_password: None,
            enable_fault_detection: false,
            enable_event_enrollment: true,
            event_enrollment_interval_secs: 10,
            discovery_policy: DiscoveryPolicy::default(),
            cov_policy: CovPolicy::default(),
            request_admission_policy: RequestAdmissionPolicy::default(),
        }
    }
}
