//! BACnet/IP over UDP transport (Annex J).
//!
//! Wraps a tokio UDP socket with BVLL framing. The recv loop decodes
//! incoming BVLL frames and extracts NPDU bytes + source MAC for the
//! network layer. Optionally acts as a BBMD or foreign device.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::bbmd::{self, BbmdState, BdtEntry, FdtEntryWire};
pub use crate::bbmd::{FdtCounters, ForeignDevicePolicy};
use crate::bvll::{decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, BvllMessage};
use crate::port::{ReceivedNpdu, TransportPort};
use crate::udp_metadata::{DestinationReceiver, IpVersion};
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;

mod control;
mod fanout;
mod foreign_device;
mod io;
mod policy_bridge;
mod rate_limit;
pub use control::{BbmdControl, BbmdControlError, BbmdLifecycle, BbmdSnapshot};
pub use fanout::{FanoutCounters, FanoutPolicy};
pub use foreign_device::{
    ForeignDeviceRegistrationHandle, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus,
};
use io::{handle_bvll_message, original_destination_matches, resolve_local_ip, RecvContext};
pub use policy_bridge::{
    evaluate_narrowing_policy, BvllPolicy, BvllPolicyContext, BvllPolicyVerdict,
    EvaluatedBvllPolicy,
};
pub use rate_limit::ManagementCounters;
use rate_limit::ManagementRateLimiter;

/// Default BACnet/IP port (0xBAC0 = 47808).
pub const DEFAULT_BACNET_PORT: u16 = 0xBAC0;

/// Resolve the OS interface name and index that owns an IPv4 address.
///
/// B/IP sockets bind to `INADDR_ANY` so subnet broadcasts remain visible. A
/// kernel interface binding prevents two shared-port attachments from
/// receiving traffic for one another's interfaces.
#[allow(unsafe_code)]
#[cfg(unix)]
fn resolve_ipv4_interface(addr: Ipv4Addr) -> Option<(Vec<u8>, u32)> {
    use std::ffi::CStr;

    struct IfAddrsGuard(*mut libc::ifaddrs);

    impl Drop for IfAddrsGuard {
        fn drop(&mut self) {
            // SAFETY: getifaddrs allocated this list and this guard owns it.
            unsafe { libc::freeifaddrs(self.0) }
        }
    }

    // SAFETY: pointers are null-checked, sockaddr_in is read only for AF_INET,
    // and interface names are kernel-provided NUL-terminated strings.
    unsafe {
        let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return None;
        }
        let _guard = IfAddrsGuard(ifaddrs);
        let mut cursor = ifaddrs;
        while !cursor.is_null() {
            let ifa = &*cursor;
            if !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family as i32 == libc::AF_INET {
                let socket_addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let candidate = Ipv4Addr::from(u32::from_be(socket_addr.sin_addr.s_addr));
                if candidate == addr {
                    let name = CStr::from_ptr(ifa.ifa_name);
                    let index = libc::if_nametoindex(name.as_ptr());
                    if index != 0 {
                        return Some((name.to_bytes().to_vec(), index));
                    }
                }
            }
            cursor = ifa.ifa_next;
        }
        None
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
))]
fn ensure_reuse_port_supported(_enabled: bool) -> Result<(), Error> {
    Ok(())
}

#[cfg(not(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
)))]
fn ensure_reuse_port_supported(enabled: bool) -> Result<(), Error> {
    if enabled {
        Err(Error::Transport(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SO_REUSEPORT is unsupported on this platform",
        )))
    } else {
        Ok(())
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
))]
fn set_socket_reuse_port(socket: &socket2::Socket, enabled: bool) -> std::io::Result<()> {
    socket.set_reuse_port(enabled)
}

#[cfg(not(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
)))]
fn set_socket_reuse_port(_socket: &socket2::Socket, enabled: bool) -> std::io::Result<()> {
    if enabled {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SO_REUSEPORT is unsupported on this platform",
        ))
    } else {
        Ok(())
    }
}

// Linux permits port sharing when every participant sets SO_REUSEADDR even
// without SO_REUSEPORT, so both options are coupled to the explicit opt-in.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn set_socket_reuse_address(socket: &socket2::Socket, reuse_port: bool) -> std::io::Result<()> {
    socket.set_reuse_address(reuse_port)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn set_socket_reuse_address(socket: &socket2::Socket, _reuse_port: bool) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, WSAGetLastError, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
    };

    socket.set_reuse_address(false)?;
    let enabled: i32 = 1;
    // SAFETY: the socket is live and the option value has the expected size
    // for the duration of this synchronous call.
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            (&enabled as *const i32).cast(),
            std::mem::size_of_val(&enabled) as i32,
        )
    };
    if result == SOCKET_ERROR {
        // SAFETY: setsockopt just failed on this thread.
        Err(std::io::Error::from_raw_os_error(unsafe {
            WSAGetLastError()
        }))
    } else {
        Ok(())
    }
}

// Apple platforms need SO_REUSEADDR for broadcast binds; SO_REUSEPORT gates
// sharing the exact wildcard address and port. Other supported Unix targets
// retain the established broadcast socket behavior.
#[cfg(not(any(target_os = "android", target_os = "linux", windows)))]
fn set_socket_reuse_address(socket: &socket2::Socket, _reuse_port: bool) -> std::io::Result<()> {
    socket.set_reuse_address(true)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BvlcResponseKind {
    WriteBroadcastDistributionTableResult,
    RegisterForeignDeviceResult,
    DeleteForeignDeviceTableEntryResult,
    ReadBroadcastDistributionTableAck,
    ReadForeignDeviceTableAck,
}

impl BvlcResponseKind {
    pub(super) fn accepts(self, msg: &BvllMessage) -> bool {
        if msg.function == BvlcFunction::BVLC_RESULT {
            let Ok(code) = decode_bvlc_result_code(msg) else {
                return true;
            };
            return match self {
                Self::WriteBroadcastDistributionTableResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
                ),
                Self::RegisterForeignDeviceResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
                ),
                Self::DeleteForeignDeviceTableEntryResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
                ),
                Self::ReadBroadcastDistributionTableAck => {
                    code == BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
                }
                Self::ReadForeignDeviceTableAck => {
                    code == BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK
                }
            };
        }
        match self {
            Self::WriteBroadcastDistributionTableResult
            | Self::RegisterForeignDeviceResult
            | Self::DeleteForeignDeviceTableEntryResult => false,
            Self::ReadBroadcastDistributionTableAck => {
                msg.function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
            }
            Self::ReadForeignDeviceTableAck => {
                msg.function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
            }
        }
    }
}

pub(super) struct PendingBvlcResponse {
    id: u64,
    target: ([u8; 4], u16),
    expected: BvlcResponseKind,
    tx: oneshot::Sender<BvllMessage>,
}

impl PendingBvlcResponse {
    pub(super) fn matches(&self, sender: ([u8; 4], u16), msg: &BvllMessage) -> bool {
        self.target == sender && self.expected.accepts(msg)
    }
}

struct PendingBvlcCleanup {
    pending: Arc<StdMutex<Option<PendingBvlcResponse>>>,
    quarantine: Arc<StdMutex<HashMap<([u8; 4], u16), Instant>>>,
    target: ([u8; 4], u16),
    request_id: u64,
    armed: bool,
}

impl PendingBvlcCleanup {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingBvlcCleanup {
    fn drop(&mut self) {
        let mut slot = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot
            .as_ref()
            .is_some_and(|pending| pending.id == self.request_id)
        {
            *slot = None;
        }
        if self.armed {
            let quiet_until = Instant::now() + BipTransport::BVLC_RESPONSE_TIMEOUT;
            self.quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(self.target)
                .and_modify(|deadline| *deadline = (*deadline).max(quiet_until))
                .or_insert(quiet_until);
        }
    }
}

pub(super) fn expect_bvlc_function(msg: &BvllMessage, expected: BvlcFunction) -> Result<(), Error> {
    if msg.function == expected {
        Ok(())
    } else {
        Err(Error::Encoding(format!(
            "expected BVLC response {expected:?}, got {:?}",
            msg.function
        )))
    }
}

pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    expect_bvlc_function(msg, BvlcFunction::BVLC_RESULT)?;
    if msg.payload.len() != std::mem::size_of::<u16>() {
        return Err(Error::Encoding(format!(
            "BVLC-Result payload must be 2 bytes, got {}",
            msg.payload.len()
        )));
    }

    Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
        msg.payload[0],
        msg.payload[1],
    ])))
}

fn bvlc_result_error(msg: &BvllMessage) -> Error {
    match decode_bvlc_result_code(msg) {
        Ok(code) => Error::Bvlc { result_code: code },
        Err(err) => err,
    }
}

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

impl TransportPort for BipTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP transport already started",
            )));
        }
        if self
            .foreign_device
            .as_ref()
            .is_some_and(|configuration| configuration.ttl == 0)
        {
            return Err(Error::Encoding(
                "foreign-device registration TTL must be non-zero".into(),
            ));
        }

        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(Error::Transport)?;

        set_socket_reuse_address(&socket2, self.reuse_port).map_err(Error::Transport)?;
        set_socket_reuse_port(&socket2, self.reuse_port).map_err(Error::Transport)?;
        socket2.set_broadcast(true).map_err(Error::Transport)?;
        socket2.set_nonblocking(true).map_err(Error::Transport)?;

        // Validate self.interface is a real local IP before binding the real
        // socket to 0.0.0.0 below. Previously the kernel enforced this when
        // we bound directly to self.interface (EADDRNOTAVAIL on typos); now
        // we probe with a throwaway bind on an ephemeral port so a
        // misconfigured interface fails fast at startup rather than silently
        // advertising an unowned IP in I-Am replies via local_mac. See
        // tests::start_fails_on_nonlocal_interface.
        if !self.interface.is_unspecified() {
            std::net::UdpSocket::bind(SocketAddrV4::new(self.interface, 0))
                .map_err(Error::Transport)?;

            #[cfg(unix)]
            if self.reuse_port {
                if let Some((_interface_name, _interface_index)) =
                    resolve_ipv4_interface(self.interface)
                {
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    socket2
                        .bind_device(Some(&_interface_name))
                        .map_err(Error::Transport)?;

                    #[cfg(any(
                        target_os = "ios",
                        target_os = "visionos",
                        target_os = "macos",
                        target_os = "tvos",
                        target_os = "watchos",
                        target_os = "illumos",
                        target_os = "solaris"
                    ))]
                    socket2
                        .bind_device_by_index_v4(std::num::NonZeroU32::new(_interface_index))
                        .map_err(Error::Transport)?;

                    #[cfg(not(any(
                        target_os = "android",
                        target_os = "fuchsia",
                        target_os = "linux",
                        target_os = "ios",
                        target_os = "visionos",
                        target_os = "macos",
                        target_os = "tvos",
                        target_os = "watchos",
                        target_os = "illumos",
                        target_os = "solaris"
                    )))]
                    let _ = (_interface_name, _interface_index);
                }
            }
        }

        // Always bind to INADDR_ANY so subnet- and limited-broadcast packets
        // (destination 10.x.y.255 / 255.255.255.255) reach this socket.  A
        // Linux UDP socket bound to a specific interface IP only receives
        // packets whose destination IP matches the bound IP, so binding to
        // self.interface would silently drop every inbound broadcast — see
        // tests::socket_is_broadcast_capable_and_binds_inaddr_any. `self.interface`
        // is still used below for the announced local MAC (line 318), so I-Am
        // responses continue to advertise the correct source IP.
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.port);
        socket2.bind(&bind_addr.into()).map_err(Error::Transport)?;

        let destination_receiver =
            DestinationReceiver::configure(&socket2, IpVersion::V4).map_err(Error::Transport)?;

        let std_socket: std::net::UdpSocket = socket2.into();
        let socket = UdpSocket::from_std(std_socket).map_err(Error::Transport)?;

        let wildcard_bind = self.interface.is_unspecified();
        let local_ip = if wildcard_bind {
            resolve_local_ip().unwrap_or(Ipv4Addr::LOCALHOST)
        } else {
            self.interface
        };
        let local_unicast_ips = if wildcard_bind {
            crate::local_addresses::ipv4()
        } else {
            vec![local_ip]
        };
        #[cfg(unix)]
        if wildcard_bind && local_unicast_ips.is_empty() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "could not enumerate local IPv4 addresses for wildcard ingress",
            )));
        }

        let local_port = socket.local_addr().map_err(Error::Transport)?.port();
        self.port = local_port;

        self.local_mac = encode_bip_mac(local_ip.octets(), local_port);

        let socket = Arc::new(socket);
        self.socket = Some(Arc::clone(&socket));

        if let Some(config) = self.bbmd_config.take() {
            let mut state = BbmdState::new(local_ip.octets(), local_port);
            // Try loading persisted BDT; fall back to initial config BDT on
            // missing/unreadable files, structural decode failure, or semantic
            // validation/conflict failure. A valid persisted BDT wins; an
            // invalid configured fallback fails startup via `set_bdt` below.
            let initial_bdt = if let Some(ref path) = self.bdt_persist_path {
                match std::fs::read(path) {
                    Ok(data) => match BbmdState::decode_bdt(&data) {
                        Ok(entries) => {
                            let mut probe = BbmdState::new(local_ip.octets(), local_port);
                            match probe.set_bdt(entries) {
                                Ok(()) => {
                                    debug!(
                                        path = %path.display(),
                                        entries = probe.bdt().len(),
                                        "Loaded persisted BDT"
                                    );
                                    probe.bdt().to_vec()
                                }
                                Err(e) => {
                                    warn!(error = %e, "Persisted BDT invalid, using config");
                                    config.initial_bdt
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode persisted BDT, using config");
                            config.initial_bdt
                        }
                    },
                    Err(_) => config.initial_bdt,
                }
            } else {
                config.initial_bdt
            };
            if let Err(e) = state.set_bdt(initial_bdt) {
                return Err(Error::Encoding(format!("BDT configuration error: {e}")));
            }
            state.set_management_acl(config.management_acl);
            state.set_foreign_device_policy(config.foreign_device_policy);
            state.set_accept_foreign_devices(config.accept_foreign_devices);
            state
                .set_max_fdt_entries(config.max_fdt_entries)
                .expect("pre-start FDT capacity was validated");
            let bbmd = Arc::new(Mutex::new(state));
            if let Some(core) = &self.bbmd_control_core {
                core.attach(
                    &bbmd,
                    self.bdt_persist_path.clone(),
                    &self.management_limiter,
                    &self.fanout_counters,
                );
            }
            self.bbmd = Some(bbmd);
        }

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (npdu_tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let (fanout_tx, fanout_rx) = mpsc::channel(self.fanout_policy.queue_capacity.max(1));
        let fanout_task = tokio::spawn(fanout::run_fanout_worker(
            Arc::clone(&socket),
            fanout_rx,
            Arc::clone(&self.fanout_counters),
        ));
        self.fanout_task = Some(fanout_task);

        let fanout_dispatcher = fanout::FanoutDispatcher::new(
            fanout_tx,
            Arc::clone(&self.fanout_limiter),
            Arc::clone(&self.fanout_counters),
        );

        let recv_ctx = RecvContext {
            local_mac: self.local_mac,
            socket: Arc::clone(&socket),
            npdu_tx,
            bbmd: self.bbmd.clone(),
            broadcast_addr: self.broadcast_address,
            broadcast_port: self.port,
            pending_bvlc_response: self.pending_bvlc_response.clone(),
            bvlc_result_quarantine: Arc::clone(&self.bvlc_result_quarantine),
            management_limiter: Arc::clone(&self.management_limiter),
            fanout: Some(fanout_dispatcher),
            bvll_policy: self.bvll_policy.clone(),
            #[cfg(test)]
            force_dbtn_forward_failure: false,
        };

        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match destination_receiver
                    .recv_from(&recv_ctx.socket, &mut recv_buf)
                    .await
                {
                    Ok(received) => {
                        let data = &recv_buf[..received.len];
                        match decode_bvll(data) {
                            Ok(msg) => {
                                if !original_destination_matches(
                                    msg.function,
                                    received.destination,
                                    local_ip,
                                    recv_ctx.broadcast_addr,
                                    &local_unicast_ips,
                                    wildcard_bind,
                                    received.os_group_delivery,
                                ) {
                                    debug!(
                                        function = msg.function.to_raw(),
                                        destination = %received.destination,
                                        "Dropping BVLL/IP destination mismatch"
                                    );
                                    continue;
                                }
                                let sender_addr =
                                    if let std::net::SocketAddr::V4(v4) = received.peer {
                                        (v4.ip().octets(), v4.port())
                                    } else {
                                        continue;
                                    };

                                handle_bvll_message(&msg, sender_addr, &recv_ctx).await;
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to decode BVLL frame");
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                        debug!(error = %e, "Dropping UDP datagram with invalid destination metadata");
                    }
                    Err(e) => {
                        warn!(error = %e, "UDP recv error");
                        break;
                    }
                }
            }
        });

        self.recv_task = Some(recv_task);

        if let Some(bbmd) = self.bbmd.clone() {
            self.bbmd_fdt_purge_task = Some(Self::spawn_bbmd_fdt_purge_task(bbmd));
        }

        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let ttl = fd.ttl;
            let sock = self.socket.as_ref().unwrap().clone();
            let handle = self
                .foreign_device_registration
                .clone()
                .expect("foreign-device configuration initializes telemetry");
            let reg_task = tokio::spawn(
                foreign_device::RegistrationWorker {
                    socket: sock,
                    bbmd_addr,
                    ttl,
                    handle,
                    pending: Arc::clone(&self.pending_bvlc_response),
                    request_lock: Arc::clone(&self.bvlc_request_lock),
                    next_request_id: Arc::clone(&self.next_bvlc_request_id),
                    quarantine: Arc::clone(&self.bvlc_result_quarantine),
                    response_timeout: Self::BVLC_RESPONSE_TIMEOUT,
                }
                .run(),
            );
            self.registration_task = Some(reg_task);
        }

        self.start_committed = true;
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(core) = &self.bbmd_control_core {
            core.stop();
        }
        for task in self.abort_background_tasks() {
            let _ = task.await;
        }
        Ok(())
    }

    fn abort(&mut self) {
        if let Some(core) = &self.bbmd_control_core {
            core.stop();
        }
        let _ = self.abort_background_tasks();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let socket = self.require_socket()?;

        let (ip, port) = decode_bip_mac(mac)?;
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let socket = self.require_socket()?;

        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let mut buf = BytesMut::with_capacity(4 + npdu.len());
            encode_bvll(
                &mut buf,
                BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                npdu,
            )?;
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        let dest = SocketAddrV4::new(self.broadcast_address, self.port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        // Clause J.1.2's B/IP broadcast address is the configured broadcast
        // IP ("all 1's in the host portion") together with this port's UDP
        // port — a broadcast IP at a different port belongs to a different
        // B/IP network and must not be folded into this link's broadcast.
        mac.len() == 6
            && mac[..4] == self.broadcast_address.octets()
            && mac[4..] == self.port.to_be_bytes()
    }
}

impl Drop for BipTransport {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod acl_tests;
#[cfg(test)]
mod bdt_persistence_tests;
#[cfg(test)]
mod dbtn_tests;
#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod fdt_tests;
#[cfg(test)]
mod forwarded_tests;
#[cfg(test)]
mod management_ack_tests;
#[cfg(test)]
mod npdu_addressing_tests;
#[cfg(test)]
mod original_tests;
#[cfg(test)]
mod rate_limit_tests;
#[cfg(test)]
mod response_amplification_tests;
#[cfg(test)]
mod socket_config_tests;
#[cfg(test)]
mod tests;
