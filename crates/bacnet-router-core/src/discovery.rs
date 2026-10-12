//! Looking for routes to unknown networks, in bounded storage.
//!
//! A router asked to reach a network it has no route to sends a
//! Who-Is-Router for it before refusing the message (Clause 6.6.3.1).
//! [`Discovery`] decides when to send one: at most once per network per
//! cooldown (5 s by default), for up to `D` networks at a time, each
//! forgotten after its age (30 s) at the next sweep. Messages that wait for
//! an answer are refused at once, not buffered, and the hosted router
//! refuses them with reject reason 1; once the answer has taught the table
//! a route, the next message takes it. Network 0 and 0xFFFF are never
//! looked for, and a cancelled `Discovery` looks for nothing more.

use crate::{Duration, Instant};

/// When a network was last looked for.
#[derive(Debug, Clone, Copy)]
struct Entry {
    network: u16,
    solicited: Instant,
}

/// Unused storage.
const NO_ENTRY: Entry = Entry {
    network: 0,
    solicited: Instant::ZERO,
};

/// The networks a router is looking for: up to `D`, sorted by number.
#[derive(Debug, Clone)]
pub struct Discovery<const D: usize> {
    entries: [Entry; D],
    len: usize,
    cooldown: Duration,
    max_age: Duration,
    cancelled: bool,
}

impl<const D: usize> Default for Discovery<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const D: usize> Discovery<D> {
    /// The hosted router's cooldown between two searches for one network.
    pub const COOLDOWN: Duration = Duration::from_secs(5);

    /// The hosted router's age at which the sweep forgets a search.
    pub const MAX_AGE: Duration = Duration::from_secs(30);

    /// Nothing looked for yet, with the default cooldown and age.
    pub const fn new() -> Self {
        Self {
            entries: [NO_ENTRY; D],
            len: 0,
            cooldown: Self::COOLDOWN,
            max_age: Self::MAX_AGE,
            cancelled: false,
        }
    }

    /// Set the cooldown and age for later calls.
    pub fn set_timing(&mut self, cooldown: Duration, max_age: Duration) {
        self.cooldown = cooldown;
        self.max_age = max_age;
    }

    /// Whether to send a Who-Is-Router for `network` now; when it says so,
    /// the search is recorded at `now`.
    ///
    /// No: when cancelled, for network 0 or 0xFFFF, within the cooldown of
    /// the last search for it, or when `D` other networks are being looked
    /// for already.
    pub fn should_solicit(&mut self, network: u16, now: Instant) -> bool {
        if self.cancelled || network == 0 || network == 0xFFFF {
            return false;
        }
        let entry = Entry {
            network,
            solicited: now,
        };
        match self.find(network) {
            Ok(index) => {
                let last = self.entries[index].solicited;
                if now.saturating_duration_since(last) < self.cooldown {
                    return false;
                }
                self.entries[index] = entry;
            }
            Err(_) if self.len == D => return false,
            Err(index) => {
                self.entries.copy_within(index..self.len, index + 1);
                self.entries[index] = entry;
                self.len += 1;
            }
        }
        true
    }

    /// Forget every search older than the age at `now`, calling `expired`
    /// with each network in ascending order, and return how many.
    pub fn expire(&mut self, now: Instant, mut expired: impl FnMut(u16)) -> usize {
        let mut kept = 0;
        for index in 0..self.len {
            let entry = self.entries[index];
            if now.saturating_duration_since(entry.solicited) > self.max_age {
                expired(entry.network);
            } else {
                self.entries[kept] = entry;
                kept += 1;
            }
        }
        let removed = self.len - kept;
        self.entries[kept..self.len].fill(NO_ENTRY);
        self.len = kept;
        removed
    }

    /// Forget every search and look for nothing more: the router stopped.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.entries[..self.len].fill(NO_ENTRY);
        self.len = 0;
    }

    /// Whether [`cancel`](Self::cancel) was called.
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// When `network` was last looked for, while that is remembered.
    pub fn last_solicited(&self, network: u16) -> Option<Instant> {
        let index = self.find(network).ok()?;
        Some(self.entries[index].solicited)
    }

    /// How many networks are being looked for.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether none are.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn find(&self, network: u16) -> Result<usize, usize> {
        self.entries[..self.len].binary_search_by_key(&network, |entry| entry.network)
    }
}
