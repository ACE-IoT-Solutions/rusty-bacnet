/// Configuration for foreign device registration.
#[derive(Debug, Clone)]
pub struct ForeignDeviceConfig {
    /// BBMD IP address to register with.
    pub bbmd_ip: Ipv4Addr,
    /// BBMD port.
    pub bbmd_port: u16,
    /// Time-to-live in seconds.
    pub ttl: u16,
}

/// Pre-start configuration for BBMD mode.
struct BbmdConfig {
    initial_bdt: Vec<BdtEntry>,
    management_acl: Vec<[u8; 4]>,
    foreign_device_policy: Option<ForeignDevicePolicy>,
    accept_foreign_devices: bool,
    max_fdt_entries: usize,
}

/// Builder for native BACnet/IP socket configuration.
#[derive(Debug, Clone)]
pub struct BipTransportBuilder {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    reuse_port: bool,
}

impl BipTransportBuilder {
    /// Create a builder with shared-port binding disabled.
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        Self {
            interface,
            port,
            broadcast_address,
            reuse_port: false,
        }
    }

    /// Opt in or out of shared-port socket options before bind.
    ///
    /// Every socket sharing an address and port must opt in. Linux couples
    /// `SO_REUSEADDR` to this setting so the disabled default stays exclusive.
    pub fn reuse_port(mut self, enabled: bool) -> Self {
        self.reuse_port = enabled;
        self
    }

    /// Build an unstarted transport.
    pub fn build(self) -> Result<BipTransport, Error> {
        let mut transport = BipTransport::new(self.interface, self.port, self.broadcast_address);
        transport.set_reuse_port(self.reuse_port)?;
        Ok(transport)
    }
}
/// BACnet/IP transport over UDP.
pub struct BipTransport {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    local_mac: [u8; 6],
    socket: Option<Arc<UdpSocket>>,
    recv_task: Option<JoinHandle<()>>,
    /// BBMD configuration before start (consumed by `start()`).
    bbmd_config: Option<BbmdConfig>,
    /// BBMD state (when acting as a BBMD, created in `start()`).
    bbmd: Option<Arc<Mutex<BbmdState>>>,
    /// Cloneable, capability-limited live BBMD administration core.
    bbmd_control_core: Option<Arc<control::BbmdControlCore>>,
    /// BBMD FDT expiry purge task.
    bbmd_fdt_purge_task: Option<JoinHandle<()>>,
    /// Foreign device config (when registered as a foreign device).
    foreign_device: Option<ForeignDeviceConfig>,
    /// Read-only live registration telemetry capability.
    foreign_device_registration: Option<ForeignDeviceRegistrationHandle>,
    /// Re-registration timer task.
    registration_task: Option<JoinHandle<()>>,
    /// Pending BVLC management response, including the expected sender and response kind.
    pending_bvlc_response: Arc<StdMutex<Option<PendingBvlcResponse>>>,
    /// Serializes requests because BVLC Result has no transaction identifier.
    bvlc_request_lock: Arc<Mutex<()>>,
    next_bvlc_request_id: Arc<AtomicU64>,
    /// Per-target quiet windows after ambiguous timeout or cancellation.
    bvlc_result_quarantine: Arc<StdMutex<HashMap<([u8; 4], u16), Instant>>>,
    /// Optional path for loading an externally provisioned persisted BDT
    /// (wire format, 10 bytes per entry) at startup. Inbound Write-BDT does
    /// not update this file.
    bdt_persist_path: Option<std::path::PathBuf>,
    /// Management request and response rate limiter.
    management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    /// Broadcast forwarding fanout policy.
    fanout_policy: FanoutPolicy,
    /// Background worker task for broadcast forwarding.
    fanout_task: Option<JoinHandle<()>>,
    /// Operational counters for broadcast forwarding fanout.
    fanout_counters: Arc<fanout::AtomicFanoutCounters>,
    /// Rate limiter for broadcast forwarding fanout.
    fanout_limiter: Arc<std::sync::Mutex<fanout::FanoutRateLimiter>>,
    /// Optional narrowing-only BVLL extension policy.
    bvll_policy: Option<Arc<dyn BvllPolicy>>,
    /// Opt-in SO_REUSEPORT setting applied before bind.
    reuse_port: bool,
    /// Set after the first successful start so socket policy cannot be mutated.
    start_committed: bool,
}

impl BipTransport {
    /// Create a new BACnet/IP transport.
    ///
    /// - `interface`: Local IP to bind (use `0.0.0.0` for all interfaces)
    /// - `port`: UDP port (default 47808 / 0xBAC0)
    /// - `broadcast_address`: Directed broadcast address (e.g., `255.255.255.255`)
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        let fanout_policy = FanoutPolicy::default();
        let fanout_limiter = Arc::new(std::sync::Mutex::new(fanout::FanoutRateLimiter::new(
            fanout_policy.clone(),
        )));
        let fanout_counters = Arc::new(fanout::AtomicFanoutCounters::default());
        Self {
            interface,
            port,
            broadcast_address,
            local_mac: [0; 6],
            socket: None,
            recv_task: None,
            bbmd_config: None,
            bbmd: None,
            bbmd_control_core: None,
            bbmd_fdt_purge_task: None,
            foreign_device: None,
            foreign_device_registration: None,
            registration_task: None,
            pending_bvlc_response: Arc::new(StdMutex::new(None)),
            bvlc_request_lock: Arc::new(Mutex::new(())),
            next_bvlc_request_id: Arc::new(AtomicU64::new(1)),
            bvlc_result_quarantine: Arc::new(StdMutex::new(HashMap::new())),
            bdt_persist_path: None,
            management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
            fanout_policy,
            fanout_task: None,
            fanout_counters,
            fanout_limiter,
            bvll_policy: None,
            reuse_port: false,
            start_committed: false,
        }
    }

    /// Create a native B/IP transport builder.
    pub fn builder(
        interface: Ipv4Addr,
        port: u16,
        broadcast_address: Ipv4Addr,
    ) -> BipTransportBuilder {
        BipTransportBuilder::new(interface, port, broadcast_address)
    }

    /// Configure opt-in shared-port behavior before the first successful start.
    ///
    /// The default is `false`. Supported Unix platforms enable SO_REUSEPORT;
    /// Linux also enables SO_REUSEADDR. An explicit interface is bound at the
    /// kernel level so shared-port attachments remain isolated by interface.
    pub fn set_reuse_port(&mut self, enabled: bool) -> Result<(), Error> {
        if self.start_committed || self.socket.is_some() || self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "SO_REUSEPORT must be configured before B/IP transport start",
            )));
        }
        ensure_reuse_port_supported(enabled)?;
        self.reuse_port = enabled;
        Ok(())
    }

    /// Enable BBMD mode with the given initial BDT.
    /// Must be called before `start()`.
    pub fn enable_bbmd(&mut self, bdt: Vec<BdtEntry>) {
        if self.bbmd_control_core.is_none() {
            self.bbmd_control_core = Some(control::BbmdControlCore::new());
        }
        self.bbmd_config = Some(BbmdConfig {
            initial_bdt: bdt,
            management_acl: Vec::new(),
            foreign_device_policy: None,
            accept_foreign_devices: true,
            max_fdt_entries: BbmdState::MAX_FDT_ENTRIES,
        });
    }

    /// Return a cloneable live-control capability after BBMD mode is configured.
    pub fn bbmd_control(&self) -> Option<BbmdControl> {
        self.bbmd_control_core.as_ref().map(BbmdControl::new)
    }

    /// Configure the initial administrative registration gate.
    pub fn set_bbmd_accept_foreign_devices(&mut self, accept: bool) -> Result<(), Error> {
        let config = self.bbmd_config.as_mut().ok_or_else(|| {
            Error::Encoding("set_bbmd_accept_foreign_devices requires enable_bbmd".into())
        })?;
        config.accept_foreign_devices = accept;
        Ok(())
    }

    /// Configure the initial bounded FDT capacity.
    pub fn set_bbmd_max_fdt_entries(&mut self, maximum: usize) -> Result<(), Error> {
        if maximum == 0 || maximum > BbmdState::MAX_FDT_ENTRIES {
            return Err(Error::Encoding(format!(
                "FDT capacity must be in 1..={} ",
                BbmdState::MAX_FDT_ENTRIES
            )));
        }
        let config = self.bbmd_config.as_mut().ok_or_else(|| {
            Error::Encoding("set_bbmd_max_fdt_entries requires enable_bbmd".into())
        })?;
        config.max_fdt_entries = maximum;
        Ok(())
    }

    /// Enable foreign device registration on this BBMD with the given policy.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn enable_foreign_device_registration(&mut self, policy: ForeignDevicePolicy) {
        if let Some(config) = &mut self.bbmd_config {
            config.foreign_device_policy = Some(policy);
        } else {
            warn!("enable_foreign_device_registration called before enable_bbmd(); policy will be ignored");
        }
    }

    /// Set foreign device registration policy on this BBMD.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn set_foreign_device_policy(&mut self, policy: ForeignDevicePolicy) {
        self.enable_foreign_device_registration(policy);
    }

    /// Set the path for loading an externally provisioned persisted BDT
    /// (wire format, 10 bytes per entry) at startup.
    /// Must be called before `start()`. Inbound Write-BDT does not update
    /// this file — no additional serialization dependencies needed.
    pub fn set_bdt_persist_path(&mut self, path: std::path::PathBuf) {
        self.bdt_persist_path = Some(path);
    }

    /// Set the management ACL for BBMD Delete-FDT-Entry.
    /// Must be called after `enable_bbmd()` and before `start()`.
    /// An empty ACL denies all Delete-FDT-Entry senders (fail closed).
    pub fn set_bbmd_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        if let Some(config) = &mut self.bbmd_config {
            config.management_acl = acl;
        } else {
            // Log a warning if called before `enable_bbmd()` so misconfiguration
            // does not fail silently.
            warn!("set_bbmd_management_acl called before enable_bbmd(); ACL will be ignored");
        }
    }

    /// Configure this transport as a foreign device.
    /// Must be called before `start()`.
    pub fn register_as_foreign_device(&mut self, config: ForeignDeviceConfig) {
        self.foreign_device_registration = Some(ForeignDeviceRegistrationHandle::new(config.ttl));
        self.foreign_device = Some(config);
    }

    /// Return a cloneable live status capability for managed registration.
    pub fn foreign_device_registration(&self) -> Option<ForeignDeviceRegistrationHandle> {
        self.foreign_device_registration.clone()
    }

    /// Get the BBMD state (if BBMD mode is enabled).
    pub fn bbmd_state(&self) -> Option<&Arc<Mutex<BbmdState>>> {
        self.bbmd.as_ref()
    }

    /// Return the operational BBMD management counters.
    pub fn management_counters(&self) -> ManagementCounters {
        match self.management_limiter.lock() {
            Ok(limiter) => limiter.counters(),
            Err(poison) => poison.into_inner().counters(),
        }
    }

    /// Return operational Foreign Device Table counters if BBMD mode is enabled.
    pub async fn fdt_counters(&self) -> Option<FdtCounters> {
        if let Some(bbmd) = &self.bbmd {
            let state = bbmd.lock().await;
            Some(state.fdt_counters())
        } else {
            None
        }
    }

    /// Return the operational broadcast forwarding fanout counters.
    pub fn fanout_counters(&self) -> FanoutCounters {
        self.fanout_counters.snapshot()
    }

    /// Set the broadcast forwarding fanout policy and rate limits.
    pub fn set_fanout_policy(&mut self, policy: FanoutPolicy) {
        let policy = policy.sanitized();
        if let Ok(mut limiter) = self.fanout_limiter.lock() {
            limiter.set_policy(policy.clone());
        }
        self.fanout_policy = policy;
    }

    /// Install an optional narrowing-only BVLL policy before start.
    /// Native structural, source, quota, and fanout gates retain precedence.
    pub fn set_bvll_policy(&mut self, policy: Arc<dyn BvllPolicy>) -> Result<(), Error> {
        if self.start_committed || self.recv_task.is_some() {
            return Err(Error::Encoding(
                "BVLL policy must be configured before transport start".into(),
            ));
        }
        self.bvll_policy = Some(policy);
        Ok(())
    }

    /// Timeout for BVLC management response waiting.
    const BVLC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

    #[cfg(not(test))]
    const BBMD_FDT_PURGE_INTERVAL: Duration = Duration::from_secs(1);

    #[cfg(test)]
    const BBMD_FDT_PURGE_INTERVAL: Duration = Duration::from_millis(20);

    /// Get the socket, returning an error if not started.
    fn require_socket(&self) -> Result<&Arc<UdpSocket>, Error> {
        self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })
    }

    fn spawn_bbmd_fdt_purge_task(bbmd: Arc<Mutex<BbmdState>>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Self::BBMD_FDT_PURGE_INTERVAL);
            loop {
                ticker.tick().await;
                let purged = {
                    let mut state = bbmd.lock().await;
                    state.purge_expired()
                };
                if purged > 0 {
                    debug!(purged, "Purged expired BBMD FDT entries");
                }
            }
        })
    }

    fn abort_background_tasks(&mut self) -> Vec<JoinHandle<()>> {
        let mut tasks = Vec::new();
        if let Some(task) = self.registration_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.bbmd_fdt_purge_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.fanout_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.recv_task.take() {
            task.abort();
            tasks.push(task);
        }
        self.socket = None;
        tasks
    }

    /// Send a raw BVLC management request and await the response.
    async fn bvlc_request(
        &self,
        target: &[u8],
        function: BvlcFunction,
        expected_response: BvlcResponseKind,
        payload: &[u8],
    ) -> Result<BvllMessage, Error> {
        let socket = self.require_socket()?;
        let (ip, port) = decode_bip_mac(target)?;
        let target = (ip, port);

        // BVLC Result has no transaction identifier or echoed request
        // function. After timeout/cancellation, drain late Results during a
        // target-local quiet window before assigning a new request owner.
        let _request_guard = loop {
            let quiet_until = self
                .bvlc_result_quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&target)
                .copied();
            if let Some(deadline) = quiet_until.filter(|deadline| *deadline > Instant::now()) {
                tokio::time::sleep_until(deadline).await;
                continue;
            }

            let request_guard = self.bvlc_request_lock.lock().await;
            let quiet_until = self
                .bvlc_result_quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&target)
                .copied();
            if quiet_until.is_some_and(|deadline| deadline > Instant::now()) {
                drop(request_guard);
                continue;
            }
            self.bvlc_result_quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&target);
            break request_guard;
        };

        let request_id = self.next_bvlc_request_id.fetch_add(1, Ordering::Relaxed);
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let (tx, rx) = oneshot::channel();
        {
            let mut slot = self
                .pending_bvlc_response
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if slot.is_some() {
                return Err(Error::Encoding(
                    "BVLC management request already in flight".into(),
                ));
            }
            *slot = Some(PendingBvlcResponse {
                id: request_id,
                target,
                expected: expected_response,
                tx,
            });
        }
        let mut cleanup = PendingBvlcCleanup {
            pending: Arc::clone(&self.pending_bvlc_response),
            quarantine: Arc::clone(&self.bvlc_result_quarantine),
            target,
            request_id,
            armed: true,
        };

        let mut buf = BytesMut::with_capacity(4 + payload.len());
        if let Err(err) = encode_bvll(&mut buf, function, payload) {
            cleanup.disarm();
            return Err(err);
        }
        if let Err(err) = socket.send_to(&buf, dest).await {
            cleanup.disarm();
            return Err(Error::Transport(err));
        }

        match tokio::time::timeout(Self::BVLC_RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(msg)) => {
                cleanup.disarm();
                Ok(msg)
            }
            Ok(Err(_)) => {
                cleanup.disarm();
                Err(Error::Encoding("BVLC response channel dropped".to_string()))
            }
            Err(_) => Err(Error::Timeout(Self::BVLC_RESPONSE_TIMEOUT)),
        }
    }

    /// Send Read-Broadcast-Distribution-Table and return the response entries.
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, Error> {
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
                BvlcResponseKind::ReadBroadcastDistributionTableAck,
                &[],
            )
            .await?;
        if msg.function == BvlcFunction::BVLC_RESULT {
            return Err(bvlc_result_error(&msg));
        }
        expect_bvlc_function(&msg, BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK)?;
        BbmdState::decode_bdt(&msg.payload)
    }

    /// Send Write-Broadcast-Distribution-Table and return the result code.
    ///
    /// Outbound client helper only. A conforming 135-2020 receiver answers
    /// with the not-supported result and leaves its table unchanged.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[BdtEntry],
    ) -> Result<BvlcResultCode, Error> {
        let mut payload = BytesMut::with_capacity(entries.len() * bbmd::BDT_ENTRY_SIZE);
        bbmd::encode_bdt_entries(entries, &mut payload);
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
                BvlcResponseKind::WriteBroadcastDistributionTableResult,
                &payload,
            )
            .await?;
        decode_bvlc_result_code(&msg)
    }

    /// Send Read-Foreign-Device-Table and return the response entries.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, Error> {
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
                BvlcResponseKind::ReadForeignDeviceTableAck,
                &[],
            )
            .await?;
        if msg.function == BvlcFunction::BVLC_RESULT {
            return Err(bvlc_result_error(&msg));
        }
        expect_bvlc_function(&msg, BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK)?;
        bbmd::decode_fdt(&msg.payload)
    }

    /// Send Delete-Foreign-Device-Table-Entry and return the result code.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<BvlcResultCode, Error> {
        let mut payload = BytesMut::with_capacity(6);
        payload.extend_from_slice(&ip);
        payload.extend_from_slice(&port.to_be_bytes());
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
                BvlcResponseKind::DeleteForeignDeviceTableEntryResult,
                &payload,
            )
            .await?;
        decode_bvlc_result_code(&msg)
    }

    /// Send a Register-Foreign-Device BVLC message to a BBMD and return the result code.
    ///
    /// This is a low-level BVLC management operation. It does NOT configure this
    /// transport as a foreign device for broadcast behavior (use
    /// [`register_as_foreign_device`] before `start()` for that).
    pub async fn register_foreign_device_bvlc(
        &self,
        target: &[u8],
        ttl: u16,
    ) -> Result<BvlcResultCode, Error> {
        let payload = ttl.to_be_bytes();
        let msg = self
            .bvlc_request(
                target,
                BvlcFunction::REGISTER_FOREIGN_DEVICE,
                BvlcResponseKind::RegisterForeignDeviceResult,
                &payload,
            )
            .await?;
        decode_bvlc_result_code(&msg)
    }
}
