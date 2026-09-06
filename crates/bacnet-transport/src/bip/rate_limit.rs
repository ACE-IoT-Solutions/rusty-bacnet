//! Bounded inbound BBMD management rate limiter.
//!
//! Transport-local, fixed-window quota for the four covered inbound BVLC
//! management requests. Private to B/IP receive handling; no public
//! configuration, no counters, and no effect on data traffic or the
//! unconditional Write-BDT not-supported answer.

use std::time::{Duration, Instant};

use bacnet_types::enums::BvlcFunction;

/// Fixed window length for management quota accounting.
pub(super) const MANAGEMENT_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Maximum covered requests admitted per source IPv4 address per window.
pub(super) const MANAGEMENT_RATE_PER_SOURCE_IP: u32 = 16;
/// Maximum covered requests admitted globally per transport per window.
pub(super) const MANAGEMENT_RATE_GLOBAL: u32 = 256;
/// Maximum distinct source IPv4 addresses tracked per window.
pub(super) const MANAGEMENT_RATE_MAX_SOURCES: usize = 256;

/// Whether an inbound BVLC function shares the combined management quota.
///
/// Covered: Read-BDT, Read-FDT, Register-Foreign-Device,
/// Delete-FDT-Entry. Everything else — including Write-BDT (which must
/// always answer not-supported), Distribute-Broadcast-To-Network, NPDU
/// data functions, BVLC-Result, Read ACKs, and unknown functions — is
/// outside the limiter.
pub(super) fn is_covered_management_request(function: BvlcFunction) -> bool {
    function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
        || function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE
        || function == BvlcFunction::REGISTER_FOREIGN_DEVICE
        || function == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
}

/// Fixed-window, strictly bounded management quota.
///
/// Tracks at most [`MANAGEMENT_RATE_MAX_SOURCES`] source IPv4 addresses in
/// a preallocated array; the window reset clears accounting without
/// allocating. Time is supplied by the caller so accounting stays
/// deterministic in tests without sleeps.
#[derive(Debug)]
pub(super) struct ManagementRateLimiter {
    window_start: Option<Instant>,
    admitted: u32,
    entries: [([u8; 4], u32); MANAGEMENT_RATE_MAX_SOURCES],
    len: usize,
}

impl ManagementRateLimiter {
    pub(super) fn new() -> Self {
        Self {
            window_start: None,
            admitted: 0,
            entries: [([0, 0, 0, 0], 0); MANAGEMENT_RATE_MAX_SOURCES],
            len: 0,
        }
    }

    /// Admit (`true`) or silently discard (`false`) one covered request.
    ///
    /// Keyed by source IPv4 address only. Rejected requests leave all
    /// accounting unchanged and never create tracking entries.
    pub(super) fn check(&mut self, ip: [u8; 4], now: Instant) -> bool {
        match self.window_start {
            None => {
                self.window_start = Some(now);
            }
            Some(start) => {
                if now.saturating_duration_since(start) >= MANAGEMENT_RATE_WINDOW {
                    self.window_start = Some(now);
                    self.admitted = 0;
                    self.len = 0;
                }
            }
        }

        if self.admitted >= MANAGEMENT_RATE_GLOBAL {
            return false;
        }

        for i in 0..self.len {
            if self.entries[i].0 == ip {
                if self.entries[i].1 >= MANAGEMENT_RATE_PER_SOURCE_IP {
                    return false;
                }
                self.entries[i].1 += 1;
                self.admitted += 1;
                return true;
            }
        }

        if self.len >= MANAGEMENT_RATE_MAX_SOURCES {
            return false;
        }
        self.entries[self.len] = (ip, 1);
        self.len += 1;
        self.admitted += 1;
        true
    }

    /// Production entry point using monotonic time.
    pub(super) fn check_now(&mut self, ip: [u8; 4]) -> bool {
        self.check(ip, Instant::now())
    }

    #[cfg(test)]
    pub(super) fn tracked_source_count(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(super) fn admitted_in_window(&self) -> u32 {
        self.admitted
    }
}

impl Default for ManagementRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
