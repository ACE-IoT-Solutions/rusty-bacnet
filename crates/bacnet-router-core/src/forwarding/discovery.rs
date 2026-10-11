//! Fixed-capacity route discovery state for unknown destination networks.
//!
//! Forwarding deliberately does not manufacture a network-control NPDU or
//! retain the application NPDU.  Instead, the caller owns the bounded state
//! and is given a typed [`DiscoveryAction`] describing when to emit
//! Who-Is-Router-To-Network and when a source-aware refusal may be returned.

use crate::NetworkNumber;

/// A caller-owned route-discovery request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryRequest {
    /// Network for which the router is requesting an I-Am-Router response.
    pub network: NetworkNumber,
    /// Monotonic deadline after which this discovery attempt expires.
    pub deadline: u64,
}

/// Action produced while an unknown destination network is being discovered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryAction {
    /// Emit a Who-Is-Router-To-Network request for `request.network`.
    WhoIsRouter(DiscoveryRequest),
    /// Keep the original NPDU pending until `request.deadline`.
    Pending(DiscoveryRequest),
    /// No pending slot was available. The caller must apply its own bounded
    /// overload policy; the forwarding core does not turn this into a reject.
    CapacityFull {
        /// Network that could not be added to the discovery set.
        network: NetworkNumber,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingRoute {
    network: NetworkNumber,
    deadline: u64,
}

/// Caller-owned, fixed-capacity route-discovery state.
///
/// The router stores no application NPDU bytes. A caller may retain or drop
/// the original packet according to its own queue policy while this state
/// suppresses repeated Who-Is-Router requests. A route learned before the
/// deadline is cleared by [`Self::route_learned`], after which the caller can
/// retry ordinary forwarding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteDiscovery<const CAPACITY: usize> {
    pending: [Option<PendingRoute>; CAPACITY],
    timeout: u64,
}

impl<const CAPACITY: usize> RouteDiscovery<CAPACITY> {
    /// Construct empty discovery state with a bounded request lifetime.
    pub const fn new(timeout: u64) -> Self {
        Self {
            pending: [None; CAPACITY],
            timeout,
        }
    }

    /// Return the configured request lifetime.
    pub const fn timeout(&self) -> u64 {
        self.timeout
    }

    /// Number of active discovery requests.
    pub const fn pending_len(&self) -> usize {
        let mut count = 0;
        let mut index = 0;
        while index < CAPACITY {
            if self.pending[index].is_some() {
                count += 1;
            }
            index += 1;
        }
        count
    }

    /// Remove all discovery requests.
    pub fn clear(&mut self) {
        self.pending.fill(None);
    }

    /// Mark a learned route as available and clear its pending request.
    pub fn route_learned(&mut self, network: NetworkNumber) -> bool {
        let Some(slot) = self
            .pending
            .iter_mut()
            .find(|slot| slot.is_some_and(|pending| pending.network == network))
        else {
            return false;
        };
        *slot = None;
        true
    }

    /// Begin or continue bounded discovery for one unknown network.
    ///
    /// The first call returns [`DiscoveryAction::WhoIsRouter`]. Calls before
    /// its deadline return [`DiscoveryAction::Pending`]. The first call at or
    /// after the deadline returns `None` and retires the request, allowing the
    /// forwarding layer to produce its eventual source-aware rejection.
    pub(crate) fn begin(&mut self, network: NetworkNumber, now: u64) -> DiscoveryDecision {
        if let Some(slot) = self
            .pending
            .iter_mut()
            .find(|slot| slot.is_some_and(|pending| pending.network == network))
        {
            let pending = slot.expect("matching pending route");
            if now < pending.deadline {
                return DiscoveryDecision::Action(DiscoveryAction::Pending(DiscoveryRequest {
                    network,
                    deadline: pending.deadline,
                }));
            }
            *slot = None;
            return DiscoveryDecision::TimedOut;
        }

        // Reclaim unrelated expired attempts before applying the capacity
        // limit. The caller is explicitly allowed to drop the original NPDU,
        // so an expired network might never be presented again to retire its
        // own slot.
        for slot in &mut self.pending {
            if slot.is_some_and(|pending| now >= pending.deadline) {
                *slot = None;
            }
        }

        let deadline = now.saturating_add(self.timeout);
        let Some(slot) = self.pending.iter_mut().find(|slot| slot.is_none()) else {
            return DiscoveryDecision::Action(DiscoveryAction::CapacityFull { network });
        };
        *slot = Some(PendingRoute { network, deadline });
        DiscoveryDecision::Action(DiscoveryAction::WhoIsRouter(DiscoveryRequest {
            network,
            deadline,
        }))
    }
}

/// Internal result used by the forwarding layer to decide whether refusal is
/// allowed yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryDecision {
    /// Continue without a refusal and expose the action to the caller.
    Action(DiscoveryAction),
    /// The bounded request expired and a refusal may now be generated.
    TimedOut,
}
