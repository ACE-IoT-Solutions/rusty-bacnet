//! Live, serialized BBMD administration.
//!
//! The control handle is deliberately separate from BVLL ingress. It can only
//! narrow the upstream safety policy, and all mutations pass through
//! [`BbmdState`] validation before becoming visible.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};

use bytes::BytesMut;
use tokio::sync::Mutex;

use crate::bbmd::{encode_bdt_entries, BbmdState, BdtEntry, FdtCounters, FdtEntryWire};

use super::fanout::{AtomicFanoutCounters, FanoutCounters};
use super::rate_limit::{ManagementCounters, ManagementRateLimiter};

const NOT_STARTED: u8 = 0;
const RUNNING: u8 = 1;
const STOPPED: u8 = 2;

/// Maximum number of source addresses accepted by the live management ACL.
pub const MAX_MANAGEMENT_ACL_ENTRIES: usize = 256;

/// Lifecycle of a cloneable BBMD control capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BbmdLifecycle {
    NotStarted,
    Running,
    Stopped,
}

/// Typed live-control failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BbmdControlError {
    NotStarted,
    Stopped,
    Invalid(String),
    Io(String),
}

impl fmt::Display for BbmdControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted => f.write_str("BBMD has not started"),
            Self::Stopped => f.write_str("BBMD has stopped"),
            Self::Invalid(reason) => write!(f, "invalid BBMD control value: {reason}"),
            Self::Io(reason) => write!(f, "BBMD persistence failed: {reason}"),
        }
    }
}

impl std::error::Error for BbmdControlError {}

/// Owned point-in-time view of live BBMD state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BbmdSnapshot {
    pub revision: u64,
    pub bdt: Vec<BdtEntry>,
    pub fdt: Vec<FdtEntryWire>,
    pub accept_foreign_devices: bool,
    pub max_fdt_entries: usize,
    pub management_acl: Vec<[u8; 4]>,
    /// Inbound Write-BDT is intentionally not enabled by this control API.
    pub wire_bdt_writes_enabled: bool,
    pub fdt_counters: FdtCounters,
    /// Upstream fixed-quota management counters.
    pub management_counters: ManagementCounters,
    /// Upstream deduplicated/budgeted fanout counters.
    pub fanout_counters: FanoutCounters,
    pub bdt_persist_path: Option<PathBuf>,
}

pub(super) struct BbmdControlCore {
    lifecycle: AtomicU8,
    revision: AtomicU64,
    state: StdMutex<Weak<Mutex<BbmdState>>>,
    persist_path: StdMutex<Option<PathBuf>>,
    management_limiter: StdMutex<Weak<std::sync::Mutex<ManagementRateLimiter>>>,
    fanout_counters: StdMutex<Weak<AtomicFanoutCounters>>,
    operations: Mutex<()>,
}

impl BbmdControlCore {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            lifecycle: AtomicU8::new(NOT_STARTED),
            revision: AtomicU64::new(0),
            state: StdMutex::new(Weak::new()),
            persist_path: StdMutex::new(None),
            management_limiter: StdMutex::new(Weak::new()),
            fanout_counters: StdMutex::new(Weak::new()),
            operations: Mutex::new(()),
        })
    }

    pub(super) fn attach(
        &self,
        state: &Arc<Mutex<BbmdState>>,
        persist_path: Option<PathBuf>,
        management_limiter: &Arc<std::sync::Mutex<ManagementRateLimiter>>,
        fanout_counters: &Arc<AtomicFanoutCounters>,
    ) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = Arc::downgrade(state);
        *self.persist_path.lock().unwrap_or_else(|p| p.into_inner()) = persist_path;
        *self
            .management_limiter
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Arc::downgrade(management_limiter);
        *self
            .fanout_counters
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Arc::downgrade(fanout_counters);
        self.lifecycle.store(RUNNING, Ordering::Release);
    }

    pub(super) fn stop(&self) {
        self.lifecycle.store(STOPPED, Ordering::Release);
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = Weak::new();
    }

    fn state(&self) -> Result<Arc<Mutex<BbmdState>>, BbmdControlError> {
        match self.lifecycle.load(Ordering::Acquire) {
            NOT_STARTED => Err(BbmdControlError::NotStarted),
            STOPPED => Err(BbmdControlError::Stopped),
            _ => self
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .upgrade()
                .ok_or(BbmdControlError::Stopped),
        }
    }

    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    fn advance_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }
}

/// Cloneable live BBMD administration capability.
#[derive(Clone)]
pub struct BbmdControl {
    core: Arc<BbmdControlCore>,
}

/// Cloneable capability that can observe, but cannot mutate, live BBMD state.
#[derive(Clone)]
pub struct BbmdSnapshotReader {
    control: BbmdControl,
}

impl BbmdSnapshotReader {
    /// Return the current controller lifecycle.
    pub fn lifecycle(&self) -> BbmdLifecycle {
        self.control.lifecycle()
    }

    /// Capture one point-in-time live BBMD observation.
    pub async fn snapshot(&self) -> Result<BbmdSnapshot, BbmdControlError> {
        self.control.snapshot().await
    }
}

impl BbmdControl {
    pub(super) fn new(core: &Arc<BbmdControlCore>) -> Self {
        Self {
            core: Arc::clone(core),
        }
    }

    pub fn lifecycle(&self) -> BbmdLifecycle {
        match self.core.lifecycle.load(Ordering::Acquire) {
            NOT_STARTED => BbmdLifecycle::NotStarted,
            RUNNING => BbmdLifecycle::Running,
            _ => BbmdLifecycle::Stopped,
        }
    }

    /// Derive a read-only observation capability.
    pub fn snapshot_reader(&self) -> BbmdSnapshotReader {
        BbmdSnapshotReader {
            control: self.clone(),
        }
    }

    pub async fn snapshot(&self) -> Result<BbmdSnapshot, BbmdControlError> {
        let _operation = self.core.operations.lock().await;
        let state = self.core.state()?;
        let mut state = state.lock().await;
        let fdt = state.fdt_snapshot();
        let management_counters = self
            .core
            .management_limiter
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .upgrade()
            .map(|limiter| match limiter.lock() {
                Ok(limiter) => limiter.counters(),
                Err(poison) => poison.into_inner().counters(),
            })
            .unwrap_or_default();
        let fanout_counters = self
            .core
            .fanout_counters
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .upgrade()
            .map(|counters| counters.snapshot())
            .unwrap_or_default();
        Ok(BbmdSnapshot {
            revision: self.core.revision(),
            bdt: state.bdt().to_vec(),
            fdt,
            accept_foreign_devices: state.accepts_foreign_devices(),
            max_fdt_entries: state.max_fdt_entries(),
            management_acl: state.management_acl().to_vec(),
            wire_bdt_writes_enabled: false,
            fdt_counters: state.fdt_counters(),
            management_counters,
            fanout_counters,
            bdt_persist_path: self
                .core
                .persist_path
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        })
    }

    /// Atomically replace the live BDT and, when configured, its trusted-local
    /// persistence file. Invalid candidates leave both unchanged.
    pub async fn replace_bdt(&self, entries: Vec<BdtEntry>) -> Result<u64, BbmdControlError> {
        if entries.len() > BbmdState::MAX_BDT_ENTRIES {
            return Err(BbmdControlError::Invalid("BDT exceeds hard cap".into()));
        }
        let _operation = self.core.operations.lock().await;
        let state = self.core.state()?;

        let (desired, persisted_peers) = {
            let mut state = state.lock().await;
            let previous = state.bdt().to_vec();
            state
                .set_bdt(entries)
                .map_err(|error| BbmdControlError::Invalid(error.to_string()))?;
            let desired = state.bdt().to_vec();
            let persisted_peers = state.bdt_peers_snapshot();
            state
                .set_bdt(previous)
                .expect("previously committed BDT remains valid");
            (desired, persisted_peers)
        };

        let persist_path = self
            .core
            .persist_path
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(path) = persist_path {
            tokio::task::spawn_blocking(move || persist_bdt_atomically(&path, &persisted_peers))
                .await
                .map_err(|error| BbmdControlError::Io(error.to_string()))?
                .map_err(|error| BbmdControlError::Io(error.to_string()))?;
        }

        state
            .lock()
            .await
            .set_bdt(desired)
            .expect("validated BDT remains valid");
        Ok(self.core.advance_revision())
    }

    pub async fn set_accept_foreign_devices(&self, accept: bool) -> Result<u64, BbmdControlError> {
        let _operation = self.core.operations.lock().await;
        let state = self.core.state()?;
        state.lock().await.set_accept_foreign_devices(accept);
        Ok(self.core.advance_revision())
    }

    pub async fn set_max_fdt_entries(&self, maximum: usize) -> Result<u64, BbmdControlError> {
        let _operation = self.core.operations.lock().await;
        let state = self.core.state()?;
        state
            .lock()
            .await
            .set_max_fdt_entries(maximum)
            .map_err(|error| BbmdControlError::Invalid(error.to_string()))?;
        Ok(self.core.advance_revision())
    }

    pub async fn set_management_acl(&self, acl: Vec<[u8; 4]>) -> Result<u64, BbmdControlError> {
        if acl.len() > MAX_MANAGEMENT_ACL_ENTRIES {
            return Err(BbmdControlError::Invalid(
                "management ACL exceeds hard cap".into(),
            ));
        }
        let _operation = self.core.operations.lock().await;
        let state = self.core.state()?;
        state.lock().await.set_management_acl(acl);
        Ok(self.core.advance_revision())
    }
}

fn persist_bdt_atomically(path: &Path, entries: &[BdtEntry]) -> std::io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bdt");
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut encoded = BytesMut::new();
        encode_bdt_entries(entries, &mut encoded);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::port::TransportPort;

    fn entry(last: u8) -> BdtEntry {
        BdtEntry {
            ip: [192, 0, 2, last],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        }
    }

    #[tokio::test]
    async fn control_lifecycle_and_fifo_snapshots() {
        let mut transport =
            super::super::BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.enable_bbmd(Vec::new());
        let control = transport.bbmd_control().unwrap();
        assert_eq!(control.lifecycle(), BbmdLifecycle::NotStarted);
        assert_eq!(control.snapshot().await, Err(BbmdControlError::NotStarted));

        let _rx = transport.start().await.unwrap();
        let revision = control.replace_bdt(vec![entry(9)]).await.unwrap();
        let snapshot = control.snapshot().await.unwrap();
        assert_eq!(snapshot.revision, revision);
        assert!(snapshot.bdt.contains(&entry(9)));
        assert!(!snapshot.wire_bdt_writes_enabled);

        transport.stop().await.unwrap();
        assert_eq!(control.lifecycle(), BbmdLifecycle::Stopped);
    }

    #[tokio::test]
    async fn controls_only_narrow_foreign_device_admission() {
        let mut transport =
            super::super::BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.enable_bbmd(Vec::new());
        transport.enable_foreign_device_registration(Default::default());
        let control = transport.bbmd_control().unwrap();
        let _rx = transport.start().await.unwrap();
        control.set_accept_foreign_devices(false).await.unwrap();
        control.set_max_fdt_entries(7).await.unwrap();
        let snapshot = control.snapshot().await.unwrap();
        assert!(!snapshot.accept_foreign_devices);
        assert_eq!(snapshot.max_fdt_entries, 7);
        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn persisted_replace_updates_disk_and_live_snapshot_together() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusty-bacnet-live-control-{}-{suffix}.bdt",
            std::process::id()
        ));
        let mut transport =
            super::super::BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.enable_bbmd(Vec::new());
        transport.set_bdt_persist_path(path.clone());
        let control = transport.bbmd_control().unwrap();
        let _rx = transport.start().await.unwrap();

        control.replace_bdt(vec![entry(8)]).await.unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(BbmdState::decode_bdt(&bytes).unwrap(), vec![entry(8)]);
        assert!(control.snapshot().await.unwrap().bdt.contains(&entry(8)));

        transport.stop().await.unwrap();
        let _ = std::fs::remove_file(path);
    }
}
