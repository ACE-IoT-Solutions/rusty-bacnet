//! The foreign device table (Annex J.5.2), in a fixed array.

use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{FdtEntries, FdtEntry};

use crate::time::{Duration, Instant};

/// Seconds a BBMD keeps a registration past its time-to-live before it
/// purges it (Annex J.5.2.3).
pub const GRACE_PERIOD_SECS: u16 = 30;

/// One foreign device's registration, as a BBMD keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    /// The foreign device's B/IP address: where the registration came from.
    pub address: SocketAddrV4,
    /// The time-to-live it registered with, in seconds.
    pub ttl: u16,
    /// When it registered or last renewed.
    pub registered_at: Instant,
}

/// An unused slot of storage.
const EMPTY_SLOT: Registration = Registration {
    address: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
    ttl: 0,
    registered_at: Instant::ZERO,
};

impl Registration {
    /// How long the registration lasts from `registered_at`: its time-to-live
    /// plus the [`GRACE_PERIOD_SECS`].
    #[must_use]
    pub const fn lifetime(&self) -> Duration {
        Duration::from_secs(self.ttl as u64 + GRACE_PERIOD_SECS as u64)
    }

    /// Whether the registration has lapsed at `now`: more than its
    /// [`lifetime`](Self::lifetime) has passed since it registered. At
    /// exactly the lifetime it is still live.
    #[must_use]
    pub fn is_expired_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.registered_at) > self.lifetime()
    }

    /// Whole seconds left at `now` before the BBMD purges the registration,
    /// grace period included, as a Read-Foreign-Device-Table-Ack reports
    /// them (Annex J.2.8.1): the lifetime in seconds less the whole seconds
    /// passed, at most 65535. It counts down from time-to-live + 30 at
    /// registration and reaches 0 one second before the registration lapses.
    #[must_use]
    pub fn seconds_remaining_at(&self, now: Instant) -> u16 {
        let lifetime_secs = u32::from(self.ttl) + u32::from(GRACE_PERIOD_SECS);
        let elapsed = now.saturating_duration_since(self.registered_at);
        // The longest lifetime is 65565 s, so a registration more than 2^17 s
        // old, where the short division stops, has nothing left.
        let left = match elapsed.whole_secs_short() {
            Some(passed) => lifetime_secs.saturating_sub(passed),
            None => 0,
        };
        u16::try_from(left).unwrap_or(u16::MAX)
    }

    /// The registration as a Read-Foreign-Device-Table-Ack row at `now`.
    #[must_use]
    pub fn wire_entry_at(&self, now: Instant) -> FdtEntry {
        FdtEntry::new(self.address, self.ttl, self.seconds_remaining_at(now))
    }
}

/// The admission rules a BBMD applies to a Register-Foreign-Device besides
/// its table's size (Annex J.5.2.2 leaves them to the BBMD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationLimits {
    /// The shortest time-to-live accepted, in seconds. A time-to-live of 0
    /// is always refused.
    pub min_ttl: u16,
    /// The longest time-to-live accepted, in seconds.
    pub max_ttl: u16,
    /// Most registrations one IPv4 address may hold, across its ports.
    pub max_entries_per_source: usize,
    /// Slots only a reserved source may fill: an ordinary source is refused
    /// once the table holds its capacity less these.
    pub reserved_capacity: usize,
}

impl Default for RegistrationLimits {
    /// Time-to-live 1 to 3600 s, 8 registrations per address, nothing
    /// reserved.
    fn default() -> Self {
        Self {
            min_ttl: 1,
            max_ttl: 3600,
            max_entries_per_source: 8,
            reserved_capacity: 0,
        }
    }
}

/// An accepted registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// The table had no entry for the address; it has one now.
    New,
    /// The address's entry restarted with the new time-to-live.
    Renewed,
}

/// Why a registration was refused. The table is unchanged apart from the
/// purge that comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationRefusal {
    /// The time-to-live is 0 or outside the limits.
    TimeToLive,
    /// The caller's own admission check said no.
    NotAdmitted,
    /// The address's IPv4 address holds its limit of registrations.
    SourceQuota,
    /// No slot is free to this source.
    TableFull,
}

impl fmt::Display for RegistrationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TimeToLive => "time-to-live outside the accepted range",
            Self::NotAdmitted => "registration not admitted",
            Self::SourceQuota => "too many registrations from this IPv4 address",
            Self::TableFull => "foreign device table full",
        })
    }
}

/// What [`Fdt::register`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct RegisterOutcome {
    /// The registration's fate.
    pub result: Result<Registered, RegistrationRefusal>,
    /// Lapsed registrations purged before the request was considered.
    pub purged: usize,
}

/// Caller storage too small for what was asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageFull;

impl fmt::Display for StorageFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no room in the storage given")
    }
}

impl core::error::Error for StorageFull {}

/// A BBMD's foreign device table: up to `N` registrations, one per B/IP
/// address, in the order they first registered.
///
/// Expiry runs on the caller's clock: every call that depends on time takes
/// `now`, and a lapsed registration stays stored, but is never forwarded to
/// or reported, until [`Fdt::purge`] or [`Fdt::register`] removes it.
#[derive(Clone)]
pub struct Fdt<const N: usize> {
    slots: [Registration; N],
    len: usize,
}

/// Shows the registrations in use, not the whole array.
impl<const N: usize> fmt::Debug for Fdt<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.registrations()).finish()
    }
}

impl<const N: usize> Default for Fdt<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Fdt<N> {
    /// Registrations the table holds at most.
    pub const CAPACITY: usize = N;

    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [EMPTY_SLOT; N],
            len: 0,
        }
    }

    /// Stored registrations, lapsed ones not yet purged included.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no registration is stored.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The stored registrations, lapsed ones not yet purged included.
    #[must_use]
    pub fn registrations(&self) -> &[Registration] {
        &self.slots[..self.len]
    }

    /// The registrations still live at `now`.
    pub fn live_at(&self, now: Instant) -> impl DoubleEndedIterator<Item = &Registration> + '_ {
        self.registrations()
            .iter()
            .filter(move |entry| !entry.is_expired_at(now))
    }

    /// The stored registration for `address`, lapsed or not.
    #[must_use]
    pub fn get(&self, address: SocketAddrV4) -> Option<&Registration> {
        self.registrations()
            .iter()
            .find(|entry| entry.address == address)
    }

    /// Whether `address` holds a registration that is live at `now`.
    #[must_use]
    pub fn is_registered_at(&self, address: SocketAddrV4, now: Instant) -> bool {
        self.get(address)
            .is_some_and(|entry| !entry.is_expired_at(now))
    }

    /// Handle a Register-Foreign-Device from `source` asking for `ttl`
    /// seconds, at `now` (Annex J.5.2.2, J.5.2.3).
    ///
    /// The steps, in order:
    ///
    /// 1. Purge the registrations lapsed at `now`.
    /// 2. Refuse a time-to-live of 0 or outside `limits`.
    /// 3. Ask `admit`, the caller's own policy (an access list, a rate
    ///    limit): `false` refuses. It is called only for a time-to-live that
    ///    passed.
    /// 4. Renew an existing registration for `source`: its timer restarts
    ///    with the new time-to-live, and no limit below applies.
    /// 5. Otherwise refuse when `source`'s IPv4 address already holds
    ///    `limits.max_entries_per_source`, or when the table has no slot for
    ///    it: an ordinary source may fill all but `limits.reserved_capacity`
    ///    slots, a `reserved` one all of them. Then append it.
    pub fn register(
        &mut self,
        source: SocketAddrV4,
        ttl: u16,
        now: Instant,
        limits: &RegistrationLimits,
        reserved: bool,
        admit: impl FnOnce() -> bool,
    ) -> RegisterOutcome {
        let purged = self.purge(now);
        let result = self.admit_and_store(source, ttl, now, limits, reserved, admit);
        RegisterOutcome { result, purged }
    }

    fn admit_and_store(
        &mut self,
        source: SocketAddrV4,
        ttl: u16,
        now: Instant,
        limits: &RegistrationLimits,
        reserved: bool,
        admit: impl FnOnce() -> bool,
    ) -> Result<Registered, RegistrationRefusal> {
        if ttl == 0 || ttl < limits.min_ttl || ttl > limits.max_ttl {
            return Err(RegistrationRefusal::TimeToLive);
        }
        if !admit() {
            return Err(RegistrationRefusal::NotAdmitted);
        }
        let registration = Registration {
            address: source,
            ttl,
            registered_at: now,
        };
        if let Some(slot) = self.slots[..self.len]
            .iter_mut()
            .find(|entry| entry.address == source)
        {
            *slot = registration;
            return Ok(Registered::Renewed);
        }
        let from_source = self
            .registrations()
            .iter()
            .filter(|entry| entry.address.ip() == source.ip())
            .count();
        if from_source >= limits.max_entries_per_source {
            return Err(RegistrationRefusal::SourceQuota);
        }
        let limit = if reserved {
            N
        } else {
            N.saturating_sub(limits.reserved_capacity)
        };
        if self.len >= limit {
            return Err(RegistrationRefusal::TableFull);
        }
        self.slots[self.len] = registration;
        self.len += 1;
        Ok(Registered::New)
    }

    /// Remove `address`'s registration, lapsed or not (Annex J.5.2.1.2);
    /// whether there was one.
    pub fn delete(&mut self, address: SocketAddrV4) -> bool {
        self.retain(|entry| entry.address != address) > 0
    }

    /// Remove the registrations lapsed at `now`; how many.
    pub fn purge(&mut self, now: Instant) -> usize {
        self.retain(|entry| !entry.is_expired_at(now))
    }

    /// Store `registration` as given, with no checks: it replaces the entry
    /// for its address, or is appended. For a caller restoring a table it
    /// saved, or placing a registration at a chosen time. Fails only when
    /// the table is full.
    pub fn restore(&mut self, registration: Registration) -> Result<(), StorageFull> {
        let address = registration.address;
        if let Some(slot) = self.slots[..self.len]
            .iter_mut()
            .find(|entry| entry.address == address)
        {
            *slot = registration;
            return Ok(());
        }
        let slot = self.slots.get_mut(self.len).ok_or(StorageFull)?;
        *slot = registration;
        self.len += 1;
        Ok(())
    }

    /// The registrations live at `now` as Read-Foreign-Device-Table-Ack rows
    /// (Annex J.2.8.1), written to `out` and returned as a table the codec
    /// encodes. Fails when `out` can't hold them all.
    pub fn wire_entries<'o>(
        &self,
        now: Instant,
        out: &'o mut [FdtEntry],
    ) -> Result<FdtEntries<'o>, StorageFull> {
        let mut written = 0;
        for entry in self.live_at(now) {
            *out.get_mut(written).ok_or(StorageFull)? = entry.wire_entry_at(now);
            written += 1;
        }
        Ok(FdtEntries::from_entries(&out[..written]))
    }

    /// Keep the registrations `keep` accepts, in order; how many went.
    fn retain(&mut self, mut keep: impl FnMut(&Registration) -> bool) -> usize {
        let mut kept = 0;
        for index in 0..self.len {
            let entry = self.slots[index];
            if keep(&entry) {
                self.slots[kept] = entry;
                kept += 1;
            }
        }
        let removed = self.len - kept;
        self.len = kept;
        removed
    }
}
