//! The hosted BBMD's table and forwarding logic as it was before it moved
//! to `bacnet-bip-core` (#1623), kept only as the reference the
//! differential tests compare the new `BbmdState` with. The bodies are the
//! old ones, unchanged but for names; the old file's comments are dropped.

use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::time::{Duration, Instant};

use bacnet_bip_codec::BvlcResultCode;
use bacnet_types::error::Error;

use super::policy::ForeignDevicePolicy;
use super::policy::{FdtCounters, ForeignDeviceRateTracker};
use super::BdtEntry;

const MAX_BDT_ENTRIES: usize = 128;
const MAX_FDT_ENTRIES: usize = 128;

#[derive(Debug, Clone)]
pub(super) struct FdtEntry {
    pub(super) ip: [u8; 4],
    pub(super) port: u16,
    pub(super) ttl: u16,
    pub(super) registered_at: Instant,
}

impl FdtEntry {
    const GRACE_PERIOD: u64 = 30;

    pub(super) fn is_expired_at(&self, now: Instant) -> bool {
        let total = Duration::from_secs(self.ttl as u64 + Self::GRACE_PERIOD);
        now.saturating_duration_since(self.registered_at) > total
    }

    pub(super) fn seconds_remaining_at(&self, now: Instant) -> u16 {
        let elapsed = now.saturating_duration_since(self.registered_at).as_secs();
        let total = self.ttl as u64 + Self::GRACE_PERIOD;
        total.saturating_sub(elapsed).min(u16::MAX as u64) as u16
    }
}

fn bdt_mask_prefix_len(mask: [u8; 4]) -> Option<u32> {
    let m = u32::from_be_bytes(mask);
    let inv = !m;
    (inv & inv.wrapping_add(1) == 0).then_some(m.count_ones())
}

fn bdt_invalid(e: &BdtEntry, reason: &str) -> Error {
    Error::Encoding(format!("BDT {reason}: {e:?}"))
}

fn validate_bdt_entry(e: &BdtEntry) -> Result<(), Error> {
    let mask = u32::from_be_bytes(e.broadcast_mask);
    let ip = u32::from_be_bytes(e.ip);
    let host = !mask;
    let prefix = bdt_mask_prefix_len(e.broadcast_mask).unwrap_or(33);
    let reason = if e.port == 0 {
        "invalid port 0"
    } else if e.ip == [0, 0, 0, 0] {
        "unspecified IP"
    } else if e.ip == [255, 255, 255, 255] {
        "limited-broadcast"
    } else if (224..=239).contains(&e.ip[0]) {
        "multicast IP"
    } else if prefix == 33 {
        "non-contiguous mask"
    } else if prefix < 8 {
        "mask prefix too wide (< /8) or limited-broadcast target"
    } else if prefix < 32 && ip & host == 0 {
        "subnet network address"
    } else if prefix < 32 && ip | mask == u32::MAX {
        "subnet directed-broadcast"
    } else if ip | host == u32::MAX {
        "limited-broadcast target"
    } else {
        ""
    };
    if reason.is_empty() {
        Ok(())
    } else {
        Err(bdt_invalid(e, reason))
    }
}

pub(super) fn canonical_bdt(entries: Vec<BdtEntry>) -> Result<Vec<BdtEntry>, Error> {
    let cap = entries.len().min(MAX_BDT_ENTRIES);
    let mut seen: HashMap<([u8; 4], u16), [u8; 4]> = HashMap::with_capacity(cap);
    let mut canonical: Vec<BdtEntry> = Vec::with_capacity(cap);
    for entry in entries {
        validate_bdt_entry(&entry)?;
        match seen.entry((entry.ip, entry.port)) {
            Entry::Occupied(o) => {
                if *o.get() != entry.broadcast_mask {
                    return Err(bdt_invalid(&entry, "conflicting masks"));
                }
            }
            Entry::Vacant(v) => {
                v.insert(entry.broadcast_mask);
                canonical.push(entry);
            }
        }
    }
    Ok(canonical)
}

fn check_bdt_capacity_with_self(rows: &[BdtEntry], ip: [u8; 4], port: u16) -> Result<(), Error> {
    let has_self = rows.iter().any(|e| e.ip == ip && e.port == port);
    let effective_len = rows.len() + usize::from(!has_self);
    if effective_len > MAX_BDT_ENTRIES {
        return Err(Error::Encoding(format!(
            "BDT size {effective_len} exceeds maximum of {} after self-entry insertion",
            MAX_BDT_ENTRIES
        )));
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct BbmdState {
    bdt: Vec<BdtEntry>,
    self_row_appended: bool,
    pub(super) fdt: Vec<FdtEntry>,
    local_ip: [u8; 4],
    local_port: u16,
    foreign_device_policy: Option<ForeignDevicePolicy>,
    rate_tracker: ForeignDeviceRateTracker,
    counters: FdtCounters,
}

impl BbmdState {
    pub(super) fn new(local_ip: [u8; 4], local_port: u16) -> Self {
        Self {
            bdt: Vec::new(),
            self_row_appended: false,
            fdt: Vec::new(),
            local_ip,
            local_port,
            foreign_device_policy: None,
            rate_tracker: ForeignDeviceRateTracker::new(),
            counters: FdtCounters::default(),
        }
    }

    pub(super) fn set_foreign_device_policy(&mut self, policy: Option<ForeignDevicePolicy>) {
        self.foreign_device_policy = policy;
    }

    pub(super) fn fdt_counters(&self) -> FdtCounters {
        self.counters
    }

    pub(super) fn local_address(&self) -> ([u8; 4], u16) {
        (self.local_ip, self.local_port)
    }

    pub(super) fn set_local_address(&mut self, ip: [u8; 4], port: u16) -> Result<(), Error> {
        if (ip, port) == (self.local_ip, self.local_port) {
            return Ok(());
        }
        check_bdt_capacity_with_self(self.configured_bdt(), ip, port)?;
        if self.self_row_appended {
            self.bdt.pop();
            self.self_row_appended = false;
        }
        self.local_ip = ip;
        self.local_port = port;
        self.ensure_self_in_bdt();
        Ok(())
    }

    pub(super) fn set_bdt(&mut self, entries: Vec<BdtEntry>) -> Result<(), Error> {
        let canonical = canonical_bdt(entries)?;
        check_bdt_capacity_with_self(&canonical, self.local_ip, self.local_port)?;
        self.bdt = canonical;
        self.self_row_appended = false;
        self.ensure_self_in_bdt();
        Ok(())
    }

    fn ensure_self_in_bdt(&mut self) {
        let has_self = self
            .bdt
            .iter()
            .any(|e| e.ip == self.local_ip && e.port == self.local_port);
        if !has_self {
            self.bdt.push(BdtEntry {
                ip: self.local_ip,
                port: self.local_port,
                broadcast_mask: [0xff, 0xff, 0xff, 0xff],
            });
            self.self_row_appended = true;
        }
    }

    pub(super) fn bdt(&self) -> &[BdtEntry] {
        &self.bdt
    }

    pub(super) fn configured_bdt(&self) -> &[BdtEntry] {
        let len = self.bdt.len() - usize::from(self.self_row_appended);
        &self.bdt[..len]
    }

    pub(super) fn register_foreign_device_at(
        &mut self,
        ip: [u8; 4],
        port: u16,
        ttl: u16,
        now: Instant,
    ) -> BvlcResultCode {
        let policy = match &self.foreign_device_policy {
            Some(p) => p.clone(),
            None => {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        };
        self.purge_expired_at(now);
        if ttl == 0 || ttl < policy.min_ttl || ttl > policy.max_ttl {
            self.counters.registrations_rejected += 1;
            return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
        }
        if let Some(allowed) = &policy.allowed_sources {
            if !allowed.contains(&ip) {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        }
        if self.rate_tracker.is_rate_exceeded(
            ip,
            now,
            policy.rate_window,
            policy.registration_rate_per_source,
            policy.registration_rate_global,
        ) {
            self.counters.registrations_rejected += 1;
            return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
        }
        let existing_idx = self.fdt.iter().position(|e| e.ip == ip && e.port == port);
        if existing_idx.is_none() {
            let current_for_ip = self.fdt.iter().filter(|e| e.ip == ip).count();
            if current_for_ip >= policy.max_entries_per_source {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
            let is_reserved = policy.reserved_sources.contains(&ip);
            let effective_limit = if is_reserved {
                MAX_FDT_ENTRIES
            } else {
                MAX_FDT_ENTRIES.saturating_sub(policy.reserved_capacity)
            };
            if self.fdt.len() >= effective_limit {
                self.counters.capacity_exhausted += 1;
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        }
        self.rate_tracker.record(ip);
        if let Some(idx) = existing_idx {
            self.fdt[idx].ttl = ttl;
            self.fdt[idx].registered_at = now;
        } else {
            self.fdt.push(FdtEntry {
                ip,
                port,
                ttl,
                registered_at: now,
            });
        }
        self.counters.registrations_accepted += 1;
        BvlcResultCode::SUCCESSFUL_COMPLETION
    }

    pub(super) fn delete_foreign_device(&mut self, ip: [u8; 4], port: u16) -> BvlcResultCode {
        let before = self.fdt.len();
        self.fdt.retain(|e| !(e.ip == ip && e.port == port));
        if self.fdt.len() < before {
            BvlcResultCode::SUCCESSFUL_COMPLETION
        } else {
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        }
    }

    pub(super) fn purge_expired_at(&mut self, now: Instant) -> usize {
        let before = self.fdt.len();
        self.fdt.retain(|e| !e.is_expired_at(now));
        let removed = before - self.fdt.len();
        self.counters.registrations_expired += removed as u64;
        removed
    }

    pub(super) fn is_bdt_peer(&self, ip: [u8; 4], port: u16) -> bool {
        self.bdt.iter().any(|e| e.ip == ip && e.port == port)
    }

    /// The old `is_registered_foreign_device`, at `now` rather than the
    /// wall clock.
    pub(super) fn is_registered_foreign_device_at(
        &mut self,
        ip: [u8; 4],
        port: u16,
        now: Instant,
    ) -> bool {
        self.purge_expired_at(now);
        self.fdt.iter().any(|e| e.ip == ip && e.port == port)
    }

    /// The old `fdt_entries_at` (tables.rs) as (ip, port, ttl, remaining).
    pub(super) fn fdt_entries_at(&self, now: Instant) -> Vec<([u8; 4], u16, u16, u16)> {
        self.fdt
            .iter()
            .filter(|entry| !entry.is_expired_at(now))
            .map(|entry| {
                (
                    entry.ip,
                    entry.port,
                    entry.ttl,
                    entry.seconds_remaining_at(now),
                )
            })
            .collect()
    }

    /// The old `fdt_wire_entries`, at `now`, as (ip, port, ttl, remaining).
    pub(super) fn fdt_wire_entries_at(&mut self, now: Instant) -> Vec<([u8; 4], u16, u16, u16)> {
        self.purge_expired_at(now);
        self.fdt
            .iter()
            .map(|entry| {
                (
                    entry.ip,
                    entry.port,
                    entry.ttl,
                    entry.seconds_remaining_at(now),
                )
            })
            .collect()
    }

    pub(super) fn forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        for entry in &self.bdt {
            if entry.ip == self.local_ip && entry.port == self.local_port {
                continue;
            }
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let directed_broadcast = [
                entry.ip[0] | !entry.broadcast_mask[0],
                entry.ip[1] | !entry.broadcast_mask[1],
                entry.ip[2] | !entry.broadcast_mask[2],
                entry.ip[3] | !entry.broadcast_mask[3],
            ];
            let target = (directed_broadcast, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
            } else {
                targets.push(target);
            }
        }
        let max_fdt_fanout = self
            .foreign_device_policy
            .as_ref()
            .map_or(32, |p| p.max_fdt_fanout);
        let mut fdt_count = 0;
        for entry in &self.fdt {
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let target = (entry.ip, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
                continue;
            }
            if fdt_count >= max_fdt_fanout {
                self.counters.fanout_budget_reached += 1;
                break;
            }
            targets.push(target);
            fdt_count += 1;
        }
        targets
    }

    pub(super) fn fdt_forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);
        let max_fdt_fanout = self
            .foreign_device_policy
            .as_ref()
            .map_or(32, |p| p.max_fdt_fanout);
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        for entry in &self.fdt {
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let target = (entry.ip, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
                continue;
            }
            if targets.len() >= max_fdt_fanout {
                self.counters.fanout_budget_reached += 1;
                break;
            }
            targets.push(target);
        }
        targets
    }
}
