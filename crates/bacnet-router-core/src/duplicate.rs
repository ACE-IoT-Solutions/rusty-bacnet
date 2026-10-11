//! Bounded routed-packet duplicate suppression.
//!
//! This primitive deliberately does not decide whether a packet is a routed
//! broadcast, where it may be forwarded, or whether a hop count is valid.
//! The router policy layer owns those decisions and calls [`DuplicateCache`]
//! only after it has built a canonical identity for the packet.

use super::LinkAddress;

/// The canonical identity of one routed NPDU for duplicate suppression.
///
/// The ingress port, immediate peer, and hop count are intentionally absent.
/// Consequently, the same packet arriving through two different router
/// interfaces has the same key when the origin and destination semantics and
/// packet fingerprint are the same. The caller must compute
/// `packet_fingerprint` from a canonical form that excludes or normalizes the
/// mutable NPDU hop count before constructing this key.
///
/// Network numbers are retained as their wire values so that global and local
/// broadcast destinations can be represented; validation of reserved values
/// belongs to the routing policy layer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DuplicateKey {
    origin_network: u16,
    origin_address: LinkAddress,
    destination_network: u16,
    destination_address: LinkAddress,
    packet_fingerprint: u64,
}

impl DuplicateKey {
    /// Construct a canonical key from routed origin/destination semantics.
    ///
    /// `origin_address` is the logical source address, not the immediate
    /// address of the interface that delivered the packet. For a broadcast
    /// destination, callers may use an empty [`LinkAddress`] or the canonical
    /// broadcast address appropriate to their data-link policy, but they must
    /// use the same representation for every ingress path.
    pub const fn new(
        origin_network: u16,
        origin_address: LinkAddress,
        destination_network: u16,
        destination_address: LinkAddress,
        packet_fingerprint: u64,
    ) -> Self {
        Self {
            origin_network,
            origin_address,
            destination_network,
            destination_address,
            packet_fingerprint,
        }
    }

    /// Return the logical origin network number.
    pub const fn origin_network(self) -> u16 {
        self.origin_network
    }

    /// Return the logical origin link address.
    pub const fn origin_address(self) -> LinkAddress {
        self.origin_address
    }

    /// Return the destination network number.
    pub const fn destination_network(self) -> u16 {
        self.destination_network
    }

    /// Return the destination link address.
    pub const fn destination_address(self) -> LinkAddress {
        self.destination_address
    }

    /// Return the caller-supplied canonical packet fingerprint.
    pub const fn packet_fingerprint(self) -> u64 {
        self.packet_fingerprint
    }
}

/// Result of offering a canonical packet identity to a duplicate cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplicateOutcome {
    /// The key was newly retained in an available cache slot.
    Inserted,
    /// The key was already retained and should be suppressed by policy.
    Duplicate,
    /// The key was retained after deterministically evicting another live key.
    Replaced,
    /// No key was retained because the cache has zero capacity or zero
    /// retention. The caller may continue forwarding, but no suppression is
    /// possible for this packet.
    Untracked,
}

/// Detached counters and occupancy for diagnostics/control interfaces.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DuplicateCacheStats {
    /// Number of currently occupied slots. Expired slots are reclaimed by
    /// [`DuplicateCache::observe`] or [`DuplicateCache::purge_expired`].
    pub occupancy: usize,
    /// Number of retained-key matches returned as [`DuplicateOutcome::Duplicate`].
    pub duplicate_hits: u64,
    /// Number of live entries displaced because the cache was full.
    pub evictions: u64,
    /// Number of entries removed after their retention deadline.
    pub expirations: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: DuplicateKey,
    expires_at: u64,
}

/// Fixed-capacity, allocation-free duplicate cache.
///
/// Entries are retained for `retention` monotonic time units. Expiration is
/// inclusive: an entry with `expires_at <= now` is expired. When all slots are
/// live, replacement uses a deterministic round-robin victim cursor. The
/// cursor starts at slot zero and advances only on replacement; expiry never
/// causes an implicit replacement.
///
/// This cache is a bounded suppression aid, not a cryptographic replay or
/// authenticity mechanism. A weak or attacker-controlled fingerprint can
/// cause false duplicate suppression or missed suppression. Use a suitably
/// strong canonical fingerprint and apply admission/authentication policy
/// outside this type.
#[derive(Clone, Debug)]
pub struct DuplicateCache<const CAPACITY: usize> {
    entries: [Option<Entry>; CAPACITY],
    retention: u64,
    replacement_cursor: usize,
    duplicate_hits: u64,
    evictions: u64,
    expirations: u64,
}

impl<const CAPACITY: usize> DuplicateCache<CAPACITY> {
    /// Construct an empty cache with the supplied monotonic retention period.
    pub const fn new(retention: u64) -> Self {
        Self {
            entries: [None; CAPACITY],
            retention,
            replacement_cursor: 0,
            duplicate_hits: 0,
            evictions: 0,
            expirations: 0,
        }
    }

    /// Return the configured retention period.
    pub const fn retention(&self) -> u64 {
        self.retention
    }

    /// Return the compile-time slot capacity.
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    /// Return the number of occupied slots, including entries not yet
    /// reclaimed because no caller has supplied a later timestamp.
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Whether no slots are currently occupied.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Offer a canonical packet identity at caller-supplied monotonic time.
    ///
    /// The hop count is intentionally not an argument: it is excluded from
    /// [`DuplicateKey`] by construction. The caller validates and decrements
    /// hop count independently before invoking the cache.
    pub fn observe(&mut self, key: DuplicateKey, now: u64) -> DuplicateOutcome {
        self.purge_expired(now);

        // A zero-retention cache is explicitly a no-suppression mode. Avoid
        // creating an entry which would already be expired at this timestamp.
        if CAPACITY == 0 || self.retention == 0 {
            return DuplicateOutcome::Untracked;
        }

        if self.entries.iter().flatten().any(|entry| entry.key == key) {
            self.duplicate_hits = self.duplicate_hits.saturating_add(1);
            return DuplicateOutcome::Duplicate;
        }

        let entry = Entry {
            key,
            expires_at: now.saturating_add(self.retention),
        };
        if let Some(slot) = self.entries.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(entry);
            return DuplicateOutcome::Inserted;
        }

        // CAPACITY == 0 returned above. Keeping the branch explicit makes the
        // invariant clear and avoids relying on modulo arithmetic with zero.
        let victim = self.replacement_cursor;
        self.entries[victim] = Some(entry);
        self.evictions = self.evictions.saturating_add(1);
        self.advance_replacement_cursor();
        DuplicateOutcome::Replaced
    }

    /// Remove entries whose retention deadline is at or before `now`.
    ///
    /// Returns the number reclaimed and increments the observable expiration
    /// counter by the same amount.
    pub fn purge_expired(&mut self, now: u64) -> usize {
        let mut removed = 0;
        for slot in &mut self.entries {
            if slot.is_some_and(|entry| entry.expires_at <= now) {
                *slot = None;
                removed += 1;
            }
        }
        self.expirations = self.expirations.saturating_add(removed as u64);
        removed
    }

    /// Return a detached diagnostic snapshot.
    pub fn stats(&self) -> DuplicateCacheStats {
        DuplicateCacheStats {
            occupancy: self.len(),
            duplicate_hits: self.duplicate_hits,
            evictions: self.evictions,
            expirations: self.expirations,
        }
    }

    /// Remove every retained key without changing diagnostic counters.
    pub fn clear(&mut self) {
        self.entries.fill(None);
        self.replacement_cursor = 0;
    }

    /// Clear diagnostic counters without changing retained keys.
    pub fn clear_stats(&mut self) {
        self.duplicate_hits = 0;
        self.evictions = 0;
        self.expirations = 0;
    }

    fn advance_replacement_cursor(&mut self) {
        let next = self.replacement_cursor.saturating_add(1);
        self.replacement_cursor = if next >= CAPACITY { 0 } else { next };
    }
}

impl<const CAPACITY: usize> Default for DuplicateCache<CAPACITY> {
    fn default() -> Self {
        Self::new(0)
    }
}
