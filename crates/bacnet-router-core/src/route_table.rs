//! Fixed-capacity direct and learned route storage.

use crate::{LinkAddress, NetworkNumber, PortId};

/// How a route entered the table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteKind {
    /// Configured directly connected network.
    Direct,
    /// Network announced by another router.
    Learned,
}

/// Current forwarding availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reachability {
    /// Traffic may be admitted.
    Reachable,
    /// Traffic is temporarily blocked until the supplied monotonic timestamp.
    BusyUntil(u64),
    /// The route is known but unavailable.
    Unreachable,
}

/// One detached route-table entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteEntry {
    /// Destination network.
    pub network: NetworkNumber,
    /// Egress port.
    pub port: PortId,
    /// Configured versus learned route.
    pub kind: RouteKind,
    /// Next-hop router MAC for learned routes; empty for direct routes.
    pub next_hop: LinkAddress,
    /// Effective reachability state.
    pub reachability: Reachability,
    /// Caller-supplied monotonic timestamp of the latest learned announcement.
    pub last_seen: Option<u64>,
}

impl RouteEntry {
    /// Return reachability with an expired busy interval treated as reachable.
    pub const fn effective_reachability(self, now: u64) -> Reachability {
        match self.reachability {
            Reachability::BusyUntil(deadline) if now >= deadline => Reachability::Reachable,
            other => other,
        }
    }
}

/// Route insertion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteTableError {
    /// The independently bounded direct-route partition is full.
    DirectTableFull,
}

/// Result of learning an advertised route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearnOutcome {
    /// Added a new learned network.
    Inserted,
    /// Refreshed or replaced an existing learned network.
    Updated,
    /// A configured direct route has precedence.
    DirectConflict,
    /// The learned-route partition is full; existing entries remain unchanged.
    TableFull,
}

/// Routing table with independent direct and learned capacity.
#[derive(Clone, Debug)]
pub struct RouteTable<const DIRECT: usize, const LEARNED: usize> {
    direct: [Option<RouteEntry>; DIRECT],
    learned: [Option<RouteEntry>; LEARNED],
}

impl<const DIRECT: usize, const LEARNED: usize> RouteTable<DIRECT, LEARNED> {
    /// Construct an empty table.
    pub const fn new() -> Self {
        Self {
            direct: [None; DIRECT],
            learned: [None; LEARNED],
        }
    }

    /// Add or replace a configured direct route.
    pub fn add_direct(
        &mut self,
        network: NetworkNumber,
        port: PortId,
    ) -> Result<(), RouteTableError> {
        if let Some(slot) = self
            .direct
            .iter_mut()
            .find(|slot| slot.is_some_and(|entry| entry.network == network))
        {
            *slot = Some(direct_entry(network, port));
            self.remove_learned(network);
            return Ok(());
        }
        let Some(slot) = self.direct.iter_mut().find(|slot| slot.is_none()) else {
            return Err(RouteTableError::DirectTableFull);
        };
        *slot = Some(direct_entry(network, port));
        self.remove_learned(network);
        Ok(())
    }

    /// Add or refresh a learned route without evicting configured networks.
    pub fn learn(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
        now: u64,
    ) -> LearnOutcome {
        if self
            .direct
            .iter()
            .flatten()
            .any(|entry| entry.network == network)
        {
            return LearnOutcome::DirectConflict;
        }
        if let Some(slot) = self
            .learned
            .iter_mut()
            .find(|slot| slot.is_some_and(|entry| entry.network == network))
        {
            *slot = Some(learned_entry(network, port, next_hop, now));
            return LearnOutcome::Updated;
        }
        let Some(slot) = self.learned.iter_mut().find(|slot| slot.is_none()) else {
            return LearnOutcome::TableFull;
        };
        *slot = Some(learned_entry(network, port, next_hop, now));
        LearnOutcome::Inserted
    }

    /// Find a route, preferring configured direct entries.
    pub fn lookup(&self, network: NetworkNumber) -> Option<RouteEntry> {
        self.direct
            .iter()
            .chain(self.learned.iter())
            .flatten()
            .copied()
            .find(|entry| entry.network == network)
    }

    /// Set a learned route temporarily busy. Direct routes are not changed.
    ///
    /// This is an administrative operation. Network-control handlers should
    /// use [`Self::mark_busy_from`] so an unrelated peer cannot mutate a route.
    pub fn mark_busy(&mut self, network: NetworkNumber, deadline: u64) -> bool {
        self.update_learned(network, |entry| {
            entry.reachability = Reachability::BusyUntil(deadline);
        })
    }

    /// Mark a learned route available.
    ///
    /// This is an administrative operation. Network-control handlers should
    /// use [`Self::mark_available_from`].
    pub fn mark_available(&mut self, network: NetworkNumber) -> bool {
        self.update_learned(network, |entry| {
            entry.reachability = Reachability::Reachable;
        })
    }

    /// Mark a learned route unreachable.
    ///
    /// This is an administrative operation. Network-control handlers should
    /// use [`Self::mark_unreachable_from`].
    pub fn mark_unreachable(&mut self, network: NetworkNumber) -> bool {
        self.update_learned(network, |entry| {
            entry.reachability = Reachability::Unreachable;
        })
    }

    /// Mark a learned route busy only when the announcing peer owns it.
    pub fn mark_busy_from(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
        deadline: u64,
    ) -> bool {
        self.update_learned_from(network, port, next_hop, |entry| {
            entry.reachability = Reachability::BusyUntil(deadline);
        })
    }

    /// Mark a learned route available only when the announcing peer owns it.
    pub fn mark_available_from(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
    ) -> bool {
        self.update_learned_from(network, port, next_hop, |entry| {
            entry.reachability = Reachability::Reachable;
        })
    }

    /// Mark a learned route unreachable only when the rejecting peer owns it.
    pub fn mark_unreachable_from(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
    ) -> bool {
        self.update_learned_from(network, port, next_hop, |entry| {
            entry.reachability = Reachability::Unreachable;
        })
    }

    /// Remove a learned route only when the requesting peer owns it.
    pub fn remove_learned_from(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
    ) -> bool {
        let Some(slot) = self.learned.iter_mut().find(|slot| {
            slot.is_some_and(|entry| {
                entry.network == network && entry.port == port && entry.next_hop == next_hop
            })
        }) else {
            return false;
        };
        *slot = None;
        true
    }

    /// Remove learned entries last seen at or before `deadline`.
    pub fn expire_learned(&mut self, deadline: u64) -> usize {
        let mut removed = 0;
        for slot in &mut self.learned {
            if slot.is_some_and(|entry| entry.last_seen.is_some_and(|seen| seen <= deadline)) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    /// Number of configured direct routes.
    pub fn direct_len(&self) -> usize {
        self.direct.iter().flatten().count()
    }

    /// Number of learned routes.
    pub fn learned_len(&self) -> usize {
        self.learned.iter().flatten().count()
    }

    /// Iterate over detached direct entries followed by learned entries.
    pub fn entries(&self) -> impl Iterator<Item = RouteEntry> + '_ {
        self.direct
            .iter()
            .chain(self.learned.iter())
            .flatten()
            .copied()
    }

    /// Iterate over configured direct routes.
    pub fn direct_entries(&self) -> impl Iterator<Item = RouteEntry> + '_ {
        self.direct.iter().flatten().copied()
    }

    /// Iterate over learned routes.
    pub fn learned_entries(&self) -> impl Iterator<Item = RouteEntry> + '_ {
        self.learned.iter().flatten().copied()
    }

    /// Mark every learned route owned by one immediate peer as busy.
    pub fn mark_all_busy_from(&mut self, port: PortId, next_hop: LinkAddress, until: u64) -> usize {
        let mut changed = 0;
        for entry in self.learned.iter_mut().flatten() {
            if entry.port == port && entry.next_hop == next_hop {
                entry.reachability = Reachability::BusyUntil(until);
                changed += 1;
            }
        }
        changed
    }

    /// Mark every learned route owned by one immediate peer as reachable.
    pub fn mark_all_available_from(&mut self, port: PortId, next_hop: LinkAddress) -> usize {
        let mut changed = 0;
        for entry in self.learned.iter_mut().flatten() {
            if entry.port == port && entry.next_hop == next_hop {
                entry.reachability = Reachability::Reachable;
                changed += 1;
            }
        }
        changed
    }

    /// Remove every learned route and return the number removed.
    pub fn clear_learned(&mut self) -> usize {
        let removed = self.learned_len();
        self.learned.fill(None);
        removed
    }

    fn remove_learned(&mut self, network: NetworkNumber) {
        if let Some(slot) = self
            .learned
            .iter_mut()
            .find(|slot| slot.is_some_and(|entry| entry.network == network))
        {
            *slot = None;
        }
    }

    fn update_learned(
        &mut self,
        network: NetworkNumber,
        update: impl FnOnce(&mut RouteEntry),
    ) -> bool {
        let Some(entry) = self
            .learned
            .iter_mut()
            .flatten()
            .find(|entry| entry.network == network)
        else {
            return false;
        };
        update(entry);
        true
    }

    fn update_learned_from(
        &mut self,
        network: NetworkNumber,
        port: PortId,
        next_hop: LinkAddress,
        update: impl FnOnce(&mut RouteEntry),
    ) -> bool {
        let Some(entry) = self.learned.iter_mut().flatten().find(|entry| {
            entry.network == network && entry.port == port && entry.next_hop == next_hop
        }) else {
            return false;
        };
        update(entry);
        true
    }
}

impl<const DIRECT: usize, const LEARNED: usize> Default for RouteTable<DIRECT, LEARNED> {
    fn default() -> Self {
        Self::new()
    }
}

fn direct_entry(network: NetworkNumber, port: PortId) -> RouteEntry {
    RouteEntry {
        network,
        port,
        kind: RouteKind::Direct,
        next_hop: LinkAddress::default(),
        reachability: Reachability::Reachable,
        last_seen: None,
    }
}

fn learned_entry(
    network: NetworkNumber,
    port: PortId,
    next_hop: LinkAddress,
    now: u64,
) -> RouteEntry {
    RouteEntry {
        network,
        port,
        kind: RouteKind::Learned,
        next_hop,
        reachability: Reachability::Reachable,
        last_seen: Some(now),
    }
}
