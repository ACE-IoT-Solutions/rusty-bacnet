//! Bounded routed-broadcast policy and deterministic fanout.
//!
//! The policy is deliberately independent of [`RouterCore`](crate::RouterCore)
//! route storage.  A caller supplies the directly connected egress ports for
//! the current routing-table snapshot and one egress queue implementation.
//! This keeps the policy useful to an embedded integration while making the
//! admission and loop-suppression contract explicit.

use crate::{
    AdmissionOutcome, DuplicateCache, DuplicateCacheStats, DuplicateKey, DuplicateOutcome, Egress,
    EgressDestination, EgressRequest, LinkAddress, NetworkNumber, PortCapabilities, PortId,
    TrafficClass,
};

/// The destination semantics of a routed broadcast request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastDestination {
    /// A broadcast on one directly connected remote BACnet network.
    Remote(NetworkNumber),
    /// A broadcast forwarded to every configured directly connected network.
    Global,
}

impl BroadcastDestination {
    const fn key_network(self) -> u16 {
        match self {
            Self::Remote(network) => network.get(),
            // 0xffff is reserved on the wire, but is only an internal key
            // discriminator here.  The destination variant remains in the
            // typed request and is never emitted as a BACnet network number.
            Self::Global => u16::MAX,
        }
    }
}

/// Explicit broadcast forwarding policy.
///
/// The default is fail-closed.  Remote and global forwarding must each be
/// enabled deliberately; enabling one does not implicitly enable the other.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BroadcastPolicyConfig {
    /// Permit `BroadcastDestination::Remote` requests.
    pub allow_remote: bool,
    /// Permit `BroadcastDestination::Global` requests.
    pub allow_global: bool,
    /// Caller-time echo retention in monotonic time units.
    pub echo_retention: u64,
}

/// Logical source and packet metadata required for canonical echo identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BroadcastRequest<'a> {
    /// Port on which the NPDU arrived.
    pub ingress_port: PortId,
    /// Logical origin network, not the immediate ingress peer.
    pub origin_network: NetworkNumber,
    /// Logical origin link address, not the immediate ingress peer.
    pub origin_address: LinkAddress,
    /// Remote or global destination semantics.
    pub destination: BroadcastDestination,
    /// Complete NPDU to be copied to each selected egress.
    pub npdu: &'a [u8],
    /// Offset of the routed hop-count octet. `None` is for hopless network
    /// controls; those messages rely entirely on bounded echo suppression.
    pub hop_count_offset: Option<usize>,
    /// Canonical packet identity computed by the caller. It must exclude or
    /// normalize the mutable hop count and must not include the immediate
    /// ingress peer.
    pub packet_fingerprint: u64,
    /// NPDU traffic class.
    pub class: TrafficClass,
    /// BACnet network priority (`0..=3`).
    pub priority: u8,
    /// Caller-supplied monotonic timestamp.
    pub now: u64,
}

/// One directly connected port considered by a broadcast fanout operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BroadcastTarget {
    /// Capabilities and directly connected network identity.
    pub capabilities: PortCapabilities,
}

/// A per-egress result. Results are written in the exact order of the target
/// slice, so callers can provide a stable port-to-diagnostics mapping without
/// allocation or sorting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BroadcastFanoutResult {
    /// Target port represented by this result.
    pub port: PortId,
    /// Policy or egress outcome for this target.
    pub outcome: BroadcastFanoutOutcome,
}

/// Per-egress broadcast outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastFanoutOutcome {
    /// The target is the ingress port and was excluded.
    IngressExcluded,
    /// Policy is disabled for this destination kind.
    PolicyDisabled,
    /// A remote request does not select this target's network.
    NotDestinationNetwork,
    /// The canonical key was seen within the configured echo window.
    Duplicate,
    /// No cache slot/retention was available, so fail-closed policy refused
    /// to forward this packet without loop protection.
    CacheUntracked,
    /// The input hop count was already zero.
    HopExhausted,
    /// The egress queue owns the rewritten NPDU.
    Accepted {
        /// Length of the copied NPDU.
        final_npdu_len: usize,
    },
    /// The egress queue has no available bounded capacity.
    Full,
    /// The egress port is unavailable.
    PortDown,
    /// The final NPDU is too large for the egress.
    Oversize {
        /// Length of the copied NPDU.
        final_npdu_len: usize,
        /// Effective egress maximum.
        maximum: usize,
    },
    /// The request expired before queue ownership.
    Expired,
    /// The adapter rejected the request as invalid.
    Invalid,
}

/// Operation-level error found before per-egress processing can begin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastError {
    /// The result buffer cannot represent one result for every target.
    ResultsTooSmall {
        /// Number of targets supplied.
        needed: usize,
        /// Result slots supplied.
        available: usize,
    },
    /// The caller scratch buffer cannot hold the copied NPDU.
    ScratchTooSmall {
        /// Input NPDU length.
        needed: usize,
        /// Scratch capacity.
        available: usize,
    },
    /// The caller supplied no NPDU bytes.
    EmptyNpdu,
    /// The hop-count offset is outside the supplied NPDU.
    InvalidHopOffset {
        /// Supplied offset.
        offset: usize,
        /// NPDU length.
        length: usize,
    },
    /// Priority is outside the BACnet four-level range.
    InvalidPriority(u8),
    /// The logical source address is absent and cannot form a stable origin.
    InvalidOrigin,
}

/// Counters and cache disposition for one fanout operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BroadcastReport {
    /// Number of targets selected after ingress/network filtering.
    pub selected: usize,
    /// Number of targets excluded because they were ingress.
    pub ingress_excluded: usize,
    /// Number of targets excluded by remote-network selection.
    pub not_destination_network: usize,
    /// Number of successful queue admissions.
    pub accepted: usize,
    /// Number of queue-full results.
    pub full: usize,
    /// Number of unavailable-port results.
    pub port_down: usize,
    /// Number of oversize results.
    pub oversize: usize,
    /// Number of expired results.
    pub expired: usize,
    /// Number of adapter-invalid results.
    pub invalid: usize,
    /// Cache disposition for the canonical packet. `None` means no target
    /// was selected or policy was disabled.
    pub cache_outcome: Option<DuplicateOutcome>,
}

impl BroadcastReport {
    const fn new() -> Self {
        Self {
            selected: 0,
            ingress_excluded: 0,
            not_destination_network: 0,
            accepted: 0,
            full: 0,
            port_down: 0,
            oversize: 0,
            expired: 0,
            invalid: 0,
            cache_outcome: None,
        }
    }
}

/// Bounded routed-broadcast policy with a fixed-capacity echo cache.
#[derive(Clone, Debug)]
pub struct BroadcastPolicy<const CACHE: usize> {
    config: BroadcastPolicyConfig,
    cache: DuplicateCache<CACHE>,
}

impl<const CACHE: usize> BroadcastPolicy<CACHE> {
    /// Construct a policy. Both destination classes are disabled when the
    /// supplied configuration is the default.
    pub const fn new(config: BroadcastPolicyConfig) -> Self {
        Self {
            cache: DuplicateCache::new(config.echo_retention),
            config,
        }
    }

    /// Return the active configuration.
    pub const fn config(&self) -> BroadcastPolicyConfig {
        self.config
    }

    /// Replace policy configuration and reset the echo cache. Resetting is
    /// deliberate: changing retention or destination scope must not preserve
    /// stale identities across policy generations.
    pub fn set_config(&mut self, config: BroadcastPolicyConfig) {
        self.config = config;
        self.cache = DuplicateCache::new(config.echo_retention);
    }

    /// Return duplicate-cache diagnostics.
    pub fn cache_stats(&self) -> DuplicateCacheStats {
        self.cache.stats()
    }

    /// Clear retained echo identities without changing the active policy.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Execute one bounded broadcast fanout.
    ///
    /// `targets` must be in the desired deterministic egress order. The
    /// method writes exactly one result for each target and calls the supplied
    /// egress at most once per selected target. A cache hit or an untracked
    /// identity produces no queue calls, preventing hopless control messages
    /// from circulating indefinitely.
    pub fn forward<E: Egress>(
        &mut self,
        request: BroadcastRequest<'_>,
        targets: &[BroadcastTarget],
        results: &mut [BroadcastFanoutResult],
        scratch: &mut [u8],
        egress: &mut E,
    ) -> Result<BroadcastReport, BroadcastError> {
        if results.len() < targets.len() {
            return Err(BroadcastError::ResultsTooSmall {
                needed: targets.len(),
                available: results.len(),
            });
        }
        if request.npdu.is_empty() {
            return Err(BroadcastError::EmptyNpdu);
        }
        if scratch.len() < request.npdu.len() {
            return Err(BroadcastError::ScratchTooSmall {
                needed: request.npdu.len(),
                available: scratch.len(),
            });
        }
        if request.priority > 3 {
            return Err(BroadcastError::InvalidPriority(request.priority));
        }
        if request.origin_address.is_empty() {
            return Err(BroadcastError::InvalidOrigin);
        }
        if let Some(offset) = request.hop_count_offset {
            if offset >= request.npdu.len() {
                return Err(BroadcastError::InvalidHopOffset {
                    offset,
                    length: request.npdu.len(),
                });
            }
        }

        let mut report = BroadcastReport::new();
        let enabled = match request.destination {
            BroadcastDestination::Remote(_) => self.config.allow_remote,
            BroadcastDestination::Global => self.config.allow_global,
        };
        for (index, target) in targets.iter().enumerate() {
            let outcome = if target.capabilities.id == request.ingress_port {
                report.ingress_excluded += 1;
                BroadcastFanoutOutcome::IngressExcluded
            } else if !enabled {
                BroadcastFanoutOutcome::PolicyDisabled
            } else if !target_selected(request.destination, target.capabilities) {
                report.not_destination_network += 1;
                BroadcastFanoutOutcome::NotDestinationNetwork
            } else {
                report.selected += 1;
                // Placeholder overwritten after validation/cache admission.
                BroadcastFanoutOutcome::HopExhausted
            };
            results[index] = BroadcastFanoutResult {
                port: target.capabilities.id,
                outcome,
            };
        }

        if !enabled || report.selected == 0 {
            return Ok(report);
        }

        if let Some(offset) = request.hop_count_offset {
            if request.npdu[offset] == 0 {
                for (index, target) in targets.iter().enumerate() {
                    if target_selected_for_request(request, target.capabilities) {
                        results[index].outcome = BroadcastFanoutOutcome::HopExhausted;
                    }
                }
                return Ok(report);
            }
        }

        let key = DuplicateKey::new(
            request.origin_network.get(),
            request.origin_address,
            request.destination.key_network(),
            LinkAddress::default(),
            request.packet_fingerprint,
        );
        let cache_outcome = self.cache.observe(key, request.now);
        report.cache_outcome = Some(cache_outcome);
        if matches!(cache_outcome, DuplicateOutcome::Duplicate) {
            for (index, target) in targets.iter().enumerate() {
                if target_selected_for_request(request, target.capabilities) {
                    results[index].outcome = BroadcastFanoutOutcome::Duplicate;
                }
            }
            return Ok(report);
        }
        if matches!(cache_outcome, DuplicateOutcome::Untracked) {
            for (index, target) in targets.iter().enumerate() {
                if target_selected_for_request(request, target.capabilities) {
                    results[index].outcome = BroadcastFanoutOutcome::CacheUntracked;
                }
            }
            return Ok(report);
        }

        scratch[..request.npdu.len()].copy_from_slice(request.npdu);
        if let Some(offset) = request.hop_count_offset {
            scratch[offset] = scratch[offset].saturating_sub(1);
        }
        for (index, target) in targets.iter().enumerate() {
            if !target_selected_for_request(request, target.capabilities) {
                continue;
            }
            let capabilities = target.capabilities;
            let npdu = &scratch[..request.npdu.len()];
            let outcome = if npdu.len() > capabilities.effective_max_npdu_len() {
                report.oversize += 1;
                BroadcastFanoutOutcome::Oversize {
                    final_npdu_len: npdu.len(),
                    maximum: capabilities.effective_max_npdu_len(),
                }
            } else {
                match egress.try_admit(EgressRequest {
                    port: capabilities.id,
                    destination: EgressDestination::Broadcast,
                    class: request.class,
                    priority: request.priority,
                    npdu,
                }) {
                    AdmissionOutcome::AcceptedOwned => {
                        report.accepted += 1;
                        BroadcastFanoutOutcome::Accepted {
                            final_npdu_len: npdu.len(),
                        }
                    }
                    AdmissionOutcome::Full => {
                        report.full += 1;
                        BroadcastFanoutOutcome::Full
                    }
                    AdmissionOutcome::Oversize => {
                        report.oversize += 1;
                        BroadcastFanoutOutcome::Oversize {
                            final_npdu_len: npdu.len(),
                            maximum: capabilities.effective_max_npdu_len(),
                        }
                    }
                    AdmissionOutcome::Expired => {
                        report.expired += 1;
                        BroadcastFanoutOutcome::Expired
                    }
                    AdmissionOutcome::PortDown => {
                        report.port_down += 1;
                        BroadcastFanoutOutcome::PortDown
                    }
                    AdmissionOutcome::Invalid => {
                        report.invalid += 1;
                        BroadcastFanoutOutcome::Invalid
                    }
                }
            };
            results[index].outcome = outcome;
        }
        Ok(report)
    }
}

impl<const CACHE: usize> Default for BroadcastPolicy<CACHE> {
    fn default() -> Self {
        Self::new(BroadcastPolicyConfig::default())
    }
}

fn target_selected(destination: BroadcastDestination, capabilities: PortCapabilities) -> bool {
    match destination {
        BroadcastDestination::Remote(network) => capabilities.network == network,
        BroadcastDestination::Global => true,
    }
}

fn target_selected_for_request(
    request: BroadcastRequest<'_>,
    capabilities: PortCapabilities,
) -> bool {
    capabilities.id != request.ingress_port && target_selected(request.destination, capabilities)
}
