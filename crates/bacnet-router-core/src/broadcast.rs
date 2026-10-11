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
    /// The supplied bytes are not a structurally valid BACnet NPDU for
    /// routed-broadcast processing.
    InvalidNpdu,
    /// The supplied hop offset does not identify the NPDU's actual hop-count
    /// field.
    HopOffsetMismatch {
        /// Caller-supplied offset.
        supplied: usize,
        /// Parsed offset, when the NPDU carries a hop count.
        actual: usize,
    },
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

        let parsed =
            ParsedBroadcastNpdu::decode(request.npdu).map_err(|_| BroadcastError::InvalidNpdu)?;
        if let Some(supplied) = request.hop_count_offset {
            match parsed.hop_offset {
                Some(actual) if actual == supplied => {}
                Some(actual) => {
                    return Err(BroadcastError::HopOffsetMismatch { supplied, actual });
                }
                None => {
                    return Err(BroadcastError::InvalidHopOffset {
                        offset: supplied,
                        length: request.npdu.len(),
                    });
                }
            }
        } else if parsed.hop_offset.is_some() {
            return Err(BroadcastError::InvalidNpdu);
        }
        match request.destination {
            BroadcastDestination::Remote(network)
                if parsed.destination_network != Some(network.get()) =>
            {
                return Err(BroadcastError::InvalidNpdu);
            }
            BroadcastDestination::Remote(_) if !parsed.destination_is_broadcast() => {
                return Err(BroadcastError::InvalidNpdu);
            }
            BroadcastDestination::Global
                if parsed.destination_network.is_some()
                    && parsed.destination_network != Some(u16::MAX) =>
            {
                return Err(BroadcastError::InvalidNpdu);
            }
            _ => {}
        }

        let final_len = parsed.final_len(request.origin_address, request.destination);
        if scratch.len() < final_len {
            return Err(BroadcastError::ScratchTooSmall {
                needed: final_len,
                available: scratch.len(),
            });
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

        if let Some(hop) = parsed.hop_count {
            if hop == 0 {
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
        // Hopless network controls have no hop budget to terminate a loop.
        // Once the bounded identity cache is live and full, replacing a live
        // entry would let A/B/C traffic continually evict one another and
        // circulate forever.  Treat replacement as fail-closed for this
        // class; ordinary routed broadcasts retain deterministic replacement.
        if parsed.hop_count.is_none() && matches!(cache_outcome, DuplicateOutcome::Replaced) {
            for (index, target) in targets.iter().enumerate() {
                if target_selected_for_request(request, target.capabilities) {
                    results[index].outcome = BroadcastFanoutOutcome::CacheUntracked;
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

        parsed.rewrite(
            request.origin_network,
            request.origin_address,
            request.destination,
            scratch,
        );
        for (index, target) in targets.iter().enumerate() {
            if !target_selected_for_request(request, target.capabilities) {
                continue;
            }
            let capabilities = target.capabilities;
            let npdu = &scratch[..final_len];
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

const BACNET_PROTOCOL_VERSION: u8 = 1;
const NETWORK_MESSAGE_BIT: u8 = 0x80;
const DESTINATION_BIT: u8 = 0x20;
const SOURCE_BIT: u8 = 0x08;

/// The validated, borrowed wire view used by the broadcast policy.  Unlike
/// the public request's optional offset, this parser owns the NPDU layout
/// decision and never trusts a caller-provided byte as a hop count.
struct ParsedBroadcastNpdu<'a> {
    control: u8,
    destination_network: Option<u16>,
    destination_fields: Option<&'a [u8]>,
    destination_mac: &'a [u8],
    source_fields: Option<&'a [u8]>,
    hop_offset: Option<usize>,
    hop_count: Option<u8>,
    remainder: &'a [u8],
}

impl<'a> ParsedBroadcastNpdu<'a> {
    fn decode(data: &'a [u8]) -> Result<Self, ()> {
        if data.len() < 2 || data[0] != BACNET_PROTOCOL_VERSION {
            return Err(());
        }
        let control = data[1];
        let mut offset = 2;
        let (destination_network, destination_fields, destination_mac) =
            if control & DESTINATION_BIT != 0 {
                let start = offset;
                if data.len() < offset + 3 {
                    return Err(());
                }
                let network = u16::from_be_bytes([data[offset], data[offset + 1]]);
                offset += 2;
                let length = data[offset] as usize;
                offset += 1;
                if length > 6 || data.len() < offset + length {
                    return Err(());
                }
                let address = &data[offset..offset + length];
                offset += length;
                (Some(network), Some(&data[start..offset]), address)
            } else {
                (None, None, &[] as &[u8])
            };
        let source_fields = if control & SOURCE_BIT != 0 {
            let start = offset;
            if data.len() < offset + 3 {
                return Err(());
            }
            let network = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            let length = data[offset] as usize;
            offset += 1;
            if network == 0 || network == u16::MAX || length == 0 || length > 6 {
                return Err(());
            }
            if data.len() < offset + length {
                return Err(());
            }
            offset += length;
            Some(&data[start..offset])
        } else {
            None
        };
        let (hop_offset, hop_count, remainder) = if destination_network.is_some() {
            let Some(&hop) = data.get(offset) else {
                return Err(());
            };
            (Some(offset), Some(hop), &data[offset + 1..])
        } else {
            (None, None, &data[offset..])
        };
        if control & NETWORK_MESSAGE_BIT != 0 {
            let Some(&message_type) = remainder.first() else {
                return Err(());
            };
            if message_type >= 0x80 && remainder.len() < 3 {
                return Err(());
            }
        }
        Ok(Self {
            control,
            destination_network,
            destination_fields,
            destination_mac,
            source_fields,
            hop_offset,
            hop_count,
            remainder,
        })
    }

    fn destination_is_broadcast(&self) -> bool {
        self.destination_mac.is_empty() || self.destination_mac == [0xff]
    }

    fn final_len(&self, origin: LinkAddress, destination: BroadcastDestination) -> usize {
        let destination_len = match destination {
            BroadcastDestination::Remote(_) => 0,
            BroadcastDestination::Global => self.destination_fields.map_or(0, <[u8]>::len),
        };
        let source_len = self
            .source_fields
            .map_or(3 + origin.as_slice().len(), <[u8]>::len);
        let hop_len = match destination {
            BroadcastDestination::Remote(_) => 0,
            BroadcastDestination::Global => usize::from(self.hop_count.is_some()),
        };
        2 + destination_len + source_len + hop_len + self.remainder.len()
    }

    fn rewrite(
        &self,
        origin_network: NetworkNumber,
        origin_address: LinkAddress,
        destination: BroadcastDestination,
        output: &mut [u8],
    ) {
        let final_delivery = matches!(destination, BroadcastDestination::Remote(_));
        output[0] = BACNET_PROTOCOL_VERSION;
        output[1] = if final_delivery {
            (self.control & !DESTINATION_BIT) | SOURCE_BIT
        } else {
            self.control | SOURCE_BIT
        };
        let mut offset = 2;
        if !final_delivery {
            match self.destination_fields {
                Some(fields) => {
                    output[offset..offset + fields.len()].copy_from_slice(fields);
                    offset += fields.len();
                }
                None => {}
            }
        }
        if let Some(fields) = self.source_fields {
            output[offset..offset + fields.len()].copy_from_slice(fields);
            offset += fields.len();
        } else {
            output[offset..offset + 2].copy_from_slice(&origin_network.get().to_be_bytes());
            offset += 2;
            output[offset] = origin_address.as_slice().len() as u8;
            offset += 1;
            output[offset..offset + origin_address.as_slice().len()]
                .copy_from_slice(origin_address.as_slice());
            offset += origin_address.as_slice().len();
        }
        if !final_delivery {
            if let Some(hop) = self.hop_count {
                output[offset] = hop.saturating_sub(1);
                offset += 1;
            }
        }
        output[offset..offset + self.remainder.len()].copy_from_slice(self.remainder);
    }
}

fn target_selected_for_request(
    request: BroadcastRequest<'_>,
    capabilities: PortCapabilities,
) -> bool {
    capabilities.id != request.ingress_port && target_selected(request.destination, capabilities)
}
