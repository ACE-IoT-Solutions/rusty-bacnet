/// Generic builder for BACnetServer with a pre-built transport.
pub struct ServerBuilder<T: TransportPort> {
    config: ServerConfig,
    db: ObjectDatabase,
    transport: Option<T>,
    configured_device_bindings: Vec<DeviceBinding>,
    apdu_observer: Option<ApduObserver>,
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Attach a bounded, passive network-layer APDU observer.
    pub fn apdu_observer(mut self, observer: ApduObserver) -> Self {
        self.apdu_observer = Some(observer);
        self
    }

    /// Declare the capabilities implemented by this caller-provided transport.
    /// Generic transports make no PICS transport claims unless this is set.
    pub fn runtime_capabilities(mut self, capabilities: crate::pics::RuntimeCapabilities) -> Self {
        self.config.runtime_capabilities = capabilities;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Set the pre-built transport.
    pub fn transport(mut self, transport: T) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
        self
    }

    /// Set the fail-closed ConfirmedAuditNotification authorization policy.
    pub fn audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Set the fail-closed UnconfirmedAuditNotification authorization policy.
    pub fn unconfirmed_audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    ///
    /// When enabled, every object's opt-in reliability hook runs every 10
    /// seconds; the default hook is a no-op.
    ///
    /// Reliability evaluation only; Event Enrollment evaluation is configured
    /// by [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes
    /// (default 10).
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
        self
    }

    /// Set the segmentation support this device advertises and enforces.
    ///
    /// The dispatch loop honors the advertisement (Clause 5.4.5.1): inbound
    /// segmented requests are reassembled only under `BOTH` or `RECEIVE`, and
    /// draw a SEGMENTATION_NOT_SUPPORTED Abort otherwise. The default is
    /// `NONE`.
    pub fn segmentation_supported(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation_supported = segmentation;
        self
    }

    /// Set the vendor identifier (used in IAm responses and protocol operations).
    pub fn vendor_id(mut self, id: u16) -> Self {
        self.config.vendor_id = id;
        self
    }

    /// Set the discovery rate-limiting and duplicate suppression policy.
    pub fn discovery_policy(mut self, policy: DiscoveryPolicy) -> Self {
        self.config.discovery_policy = policy;
        self
    }

    /// Set the COV quota and notification work budget policy.
    pub fn cov_policy(mut self, policy: CovPolicy) -> Self {
        self.config.cov_policy = policy;
        self
    }

    /// Build and start the server.
    pub async fn build(self) -> Result<BACnetServer<T>, Error> {
        let transport = self
            .transport
            .ok_or_else(|| Error::Encoding("transport not set on ServerBuilder".into()))?;
        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            self.db,
            transport,
            Some(ClockConfig::default()),
            self.configured_device_bindings,
            self.apdu_observer,
        )
        .await
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipServerBuilder {
    config: ServerConfig,
    db: ObjectDatabase,
    configured_device_bindings: Vec<DeviceBinding>,
    apdu_observer: Option<ApduObserver>,
}

impl BipServerBuilder {
    /// Attach a bounded, passive network-layer APDU observer.
    pub fn apdu_observer(mut self, observer: ApduObserver) -> Self {
        self.apdu_observer = Some(observer);
        self
    }

    /// Set the local interface IP.
    pub fn interface(mut self, ip: Ipv4Addr) -> Self {
        self.config.interface = ip;
        self
    }

    /// Set the UDP port.
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the directed broadcast address.
    pub fn broadcast_address(mut self, addr: Ipv4Addr) -> Self {
        self.config.broadcast_address = addr;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
        self
    }

    /// Set the fail-closed ConfirmedAuditNotification authorization policy.
    pub fn audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Set the fail-closed UnconfirmedAuditNotification authorization policy.
    pub fn unconfirmed_audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    ///
    /// When enabled, every object's opt-in reliability hook runs every 10
    /// seconds; the default hook is a no-op.
    ///
    /// Reliability evaluation only; Event Enrollment evaluation is configured
    /// by [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes
    /// (default 10).
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
        self
    }

    /// Set the segmentation support this device advertises and enforces.
    ///
    /// The dispatch loop honors the advertisement (Clause 5.4.5.1): inbound
    /// segmented requests are reassembled only under `BOTH` or `RECEIVE`, and
    /// draw a SEGMENTATION_NOT_SUPPORTED Abort otherwise. The default is
    /// `NONE`.
    pub fn segmentation_supported(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation_supported = segmentation;
        self
    }

    /// Set the vendor identifier advertised in I-Am responses.
    pub fn vendor_id(mut self, id: u16) -> Self {
        self.config.vendor_id = id;
        self
    }

    /// Set the discovery rate-limiting and duplicate suppression policy.
    pub fn discovery_policy(mut self, policy: DiscoveryPolicy) -> Self {
        self.config.discovery_policy = policy;
        self
    }

    /// Set the COV quota and notification work budget policy.
    pub fn cov_policy(mut self, policy: CovPolicy) -> Self {
        self.config.cov_policy = policy;
        self
    }

    /// Build and start the server, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetServer<BipTransport>, Error> {
        let transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            self.db,
            transport,
            Some(ClockConfig::default()),
            self.configured_device_bindings,
            self.apdu_observer,
        )
        .await
    }
}

/// BACnet server with APDU dispatch and service handling.
pub struct BACnetServer<T: TransportPort> {
    config: ServerConfig,
    apdu_observer: Option<ApduObserver>,
    discovery_limiter: Arc<DiscoveryLimiter>,
    /// Server-owned clock controller; absent in explicit clockless mode.
    _clock: Option<Arc<ServerClock>>,
    /// Shared network layer (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) for post-write COV/event sends).
    network: Arc<NetworkLayer<T>>,
    /// Shared object database.
    db: Arc<RwLock<ObjectDatabase>>,
    /// COV subscription table (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) to fire post-write notifications).
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    cov_counters: Arc<crate::cov::AtomicCovCounters>,
    /// Channels for routing segmented-send events to in-progress segmented sends.
    #[allow(dead_code)]
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    /// Permits that cap live segmented response sender tasks, including
    /// cancelled senders that have not yet exited a transport send.
    #[allow(dead_code)]
    seg_send_permits: Arc<Semaphore>,
    /// Operational cap of 255 concurrent confirmed COV notification workers.
    /// Invoke-ID ownership is handled by `notification_transactions`.
    cov_in_flight: Arc<Semaphore>,
    /// Legacy public TSM state and the learned DNET-to-router cache.
    server_tsm: Arc<Mutex<ServerTsm>>,
    /// Invoke-ID ownership and terminal admission for confirmed notifications.
    notification_transactions: Arc<NotificationTransactions>,
    /// Server-lifetime exact inbound ConfirmedRequest duplicate state.
    #[allow(dead_code)]
    confirmed_request_tracker: Arc<ConfirmedRequestTracker>,
    /// Shared configured and passively observed Device recipient authority.
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    /// Communication state: 0 = Enable, 1 = Disable, 2 = DisableInitiation.
    comm_state: Arc<AtomicU8>,
    /// DCC timer owner and replacement/expiry serialization boundary.
    /// Valid replacement and explicit stop abort and join before clearing it.
    dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>>,
    dcc_outcomes: Arc<dcc_outcomes::DccOutcomes>,
    dispatch_task: Option<JoinHandle<()>>,
    request_tasks: Arc<request_tasks::RequestTasks>,
    cov_purge_task: Option<JoinHandle<()>>,
    fault_detection_task: Option<JoinHandle<()>>,
    event_enrollment_task: Option<JoinHandle<()>>,
    trend_log_task: Option<JoinHandle<()>>,
    schedule_tick_task: Option<JoinHandle<()>>,
    /// One-second `Time_Delay` confirmation task for intrinsic reporting.
    intrinsic_reporting_task: Option<JoinHandle<()>>,
    /// Monotonic Binary Lighting Output WARN_OFF/WARN_RELINQUISH task.
    binary_lighting_operation_task: Option<JoinHandle<()>>,
    /// Publishes transport-owned observations to bound NetworkPort objects.
    network_port_live: network_port_live::NetworkPortLiveController,
    local_mac: MacAddr,
}

/// Cloneable handle for sending unsolicited I-Am announcements.
pub struct IAmBroadcaster<T: TransportPort> {
    config: ServerConfig,
    network: Arc<NetworkLayer<T>>,
    db: Arc<RwLock<ObjectDatabase>>,
}

impl<T: TransportPort> Clone for IAmBroadcaster<T> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            network: Arc::clone(&self.network),
            db: Arc::clone(&self.db),
        }
    }
}

impl BACnetServer<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipServerBuilder {
        let mut config = ServerConfig::default();
        config.runtime_capabilities = crate::pics::RuntimeCapabilities::bip_v4();
        BipServerBuilder {
            config,
            db: ObjectDatabase::new(),
            configured_device_bindings: Vec::new(),
            apdu_observer: None,
        }
    }

    /// Create a BIP-specific builder (alias for backward compatibility).
    pub fn builder() -> BipServerBuilder {
        Self::bip_builder()
    }
}

mod clock;
#[cfg(test)]
pub(crate) use clock::clocked_test_database;
pub use clock::ClockConfig;
use clock::ServerClock;
mod binary_lighting_lifecycle;
mod confirmed_request_tracker;
mod cov_clock;
mod cov_encoding;
mod cov_notifications;
mod cov_snapshot;
mod dcc_disable_rate;
pub(crate) mod dcc_outcomes;
mod dcc_policy;
pub use dcc_disable_rate::DccDisableRateLimit;
mod dcc_timer;
pub use dcc_outcomes::DccOutcomeCounters;
pub use dcc_policy::{DccPolicy, DccSource, DccSourceRestriction};
mod device_bindings;
mod discovery;
pub use discovery::{DiscoveryCounters, DiscoveryPolicy};
pub(crate) use discovery::{DiscoveryLimiter, PreCheckDecision, WhoHasTarget};
mod dispatch;
mod event_enrollment_lifecycle;
mod event_message_policy;
pub(crate) mod event_notification_payload;
mod event_notifications;
mod event_recipient_route;
pub(crate) mod event_timestamp;
mod lifecycle;
mod lifecycle_dispatch;
mod local_writes;
mod network_port_live;
mod notification_transactions;
mod requests;
#[cfg(feature = "sc-tls")]
mod sc_builder;
#[cfg(test)]
pub(crate) use requests::{EXECUTED_CONFIRMED, EXECUTED_UNCONFIRMED};
#[cfg(test)]
mod audit_notification_tests;
#[cfg(test)]
mod unconfirmed_audit_notification_tests;
#[cfg(feature = "sc-tls")]
pub use sc_builder::ScServerBuilder;
mod responses;
mod segmentation;
mod segmented_receive;
mod segmented_send;
pub(crate) use segmented_send::*;
mod request_admission;
mod rpm_budget;
pub use rpm_budget::ReadPropertyMultipleBudget;
mod alarm_summary_budget;
pub use alarm_summary_budget::GetAlarmSummaryBudget;
mod atomic_read_file_budget;
mod atomic_write_file_budget;
mod read_range_budget;
pub use read_range_budget::ReadRangeBudget;
mod event_information_budget;
pub use event_information_budget::GetEventInformationBudget;
mod enrollment_summary_budget;
pub use atomic_read_file_budget::AtomicReadFileBudget;
pub use atomic_write_file_budget::AtomicWriteFileBudget;
pub use enrollment_summary_budget::GetEnrollmentSummaryBudget;
#[cfg(test)]
mod atomic_read_file_tests;
#[cfg(test)]
mod atomic_write_file_tests;
#[cfg(test)]
mod enrollment_summary_tests;
mod request_peer;
mod request_tasks;
pub use request_admission::{RequestAdmissionCounters, RequestAdmissionPolicy};
mod shutdown;

#[cfg(test)]
mod acknowledge_alarm_tests;
#[cfg(test)]
mod apdu_observer_tests;
#[cfg(test)]
mod audit_log_query_tests;
#[cfg(test)]
mod binary_lighting_task_tests;
#[cfg(test)]
mod cov_budget_tests;
#[cfg(test)]
mod cov_notifications_tests;
#[cfg(test)]
mod cov_quota_tests;
#[cfg(test)]
mod dcc_event_detection_tests;
#[cfg(test)]
mod device_bindings_tests;
#[cfg(test)]
mod device_recipient_routing_tests;
#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod event_confirmed_routing_tests;
#[cfg(test)]
mod event_enable_distribution_tests;
#[cfg(test)]
mod event_enrollment_task_tests;
#[cfg(test)]
mod event_network_priority_tests;
#[cfg(test)]
mod event_notifications_tests;
#[cfg(test)]
mod event_recipient_routing_tests;
#[cfg(test)]
mod life_safety_cov_tests;
#[cfg(test)]
mod life_safety_operation_tests;
#[cfg(test)]
mod network_port_live_tests;
#[cfg(test)]
mod notification_transactions_tests;
#[cfg(test)]
mod segmentation_tests;
#[cfg(test)]
mod tests;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub fn generic_builder() -> ServerBuilder<T> {
        ServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            transport: None,
            configured_device_bindings: Vec::new(),
            apdu_observer: None,
        }
    }

    /// Get a snapshot of discovery rate-limiting counters.
    pub fn discovery_counters(&self) -> DiscoveryCounters {
        self.discovery_limiter.counters()
    }

    /// Get a snapshot of COV operational and telemetry counters.
    pub fn cov_counters(&self) -> CovCounters {
        self.cov_counters.snapshot()
    }

    /// Purge all active COV subscriptions for a peer, deterministically releasing its quota.
    pub async fn remove_peer_subscriptions(
        &self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let mut table = self.cov_table.write().await;
        table.remove_peer_subscriptions(mac, network)
    }
}
