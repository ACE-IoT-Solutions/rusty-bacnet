//! Bounded state transitions for BACnet router network-control messages.
//!
//! The wire codec in [`crate::control`] validates a complete message but does
//! not decide what that message is allowed to do to a router.  This module is
//! the stateful layer above it.  It owns no clock or transport: callers pass a
//! monotonic timestamp, a stable local [`PortId`], the immediate peer address,
//! and caller-owned output buffers.
//!
//! An Initialize-Routing-Table `port_id` is deliberately treated as an
//! opaque identifier belonging to the advertising router.  It is never copied
//! into the local route table.  Learned entries instead use the local ingress
//! port and the immediate peer address as their forwarding owner.  This keeps
//! wire port numbering from being confused with local array indexes.

use crate::control::{
    decode_control, ControlCodecError, InitializeRoutingTable, NetworkControl, NetworkList,
    RoutingTable,
};
use crate::{
    AdmissionOutcome, LearnOutcome, LinkAddress, NetworkNumber, PortId, Reachability, RouteEntry,
    RouteTable,
};

/// Default duration for a Router-Busy learned state.
pub const DEFAULT_BUSY_DURATION: u64 = 30;
/// Default lifetime of an I-Am-Router or routing-table learned route.
pub const DEFAULT_LEARNED_AGE: u64 = 300;

/// The local interpretation of the wire `port_id` field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortIdSemantics {
    /// The wire identifier belongs to the advertising peer and is advisory.
    /// Local forwarding uses the ingress [`PortId`] supplied to `handle`.
    RemoteOpaque,
}

/// A fixed caller-owned list of networks used by a response action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkSet<'a> {
    values: &'a [NetworkNumber],
}

impl<'a> NetworkSet<'a> {
    /// Number of networks in the response.
    pub const fn len(self) -> usize {
        self.values.len()
    }

    /// Whether the response contains no networks.
    pub const fn is_empty(self) -> bool {
        self.values.is_empty()
    }

    /// Return one network by index.
    pub fn get(self, index: usize) -> Option<NetworkNumber> {
        self.values.get(index).copied()
    }

    /// Return the detached response storage.
    pub const fn as_slice(self) -> &'a [NetworkNumber] {
        self.values
    }

    /// Encode the network list into a caller-provided control payload buffer.
    pub fn encode(self, out: &mut [u8]) -> Result<usize, ControlOutputError> {
        let needed = self.len().saturating_mul(2);
        if out.len() < needed {
            return Err(ControlOutputError::BufferTooSmall {
                needed,
                capacity: out.len(),
            });
        }
        for (index, network) in self.values.iter().enumerate() {
            out[index * 2..index * 2 + 2].copy_from_slice(&network.get().to_be_bytes());
        }
        Ok(needed)
    }
}

/// A detached, caller-buffered route snapshot suitable for diagnostics or an
/// Initialize-Routing-Table-ACK response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteSnapshot<'a> {
    entries: &'a [RouteEntry],
}

impl<'a> RouteSnapshot<'a> {
    /// Number of copied entries.
    pub const fn len(self) -> usize {
        self.entries.len()
    }

    /// Whether the snapshot contains no routes.
    pub const fn is_empty(self) -> bool {
        self.entries.is_empty()
    }

    /// Return copied entries; no route-table storage is borrowed.
    pub const fn as_slice(self) -> &'a [RouteEntry] {
        self.entries
    }

    /// Encode a complete Initialize-Routing-Table table body.  Port-info is
    /// intentionally empty because this portable core does not own link
    /// specific port metadata.
    pub fn encode(self, out: &mut [u8]) -> Result<usize, ControlOutputError> {
        if self.len() > u8::MAX as usize {
            return Err(ControlOutputError::TooManyEntries { count: self.len() });
        }
        let needed = 1usize.saturating_add(self.len().saturating_mul(4));
        if out.len() < needed {
            return Err(ControlOutputError::BufferTooSmall {
                needed,
                capacity: out.len(),
            });
        }
        out[0] = self.len() as u8;
        for (index, entry) in self.entries.iter().enumerate() {
            let offset = 1 + index * 4;
            out[offset..offset + 2].copy_from_slice(&entry.network.get().to_be_bytes());
            out[offset + 2] = entry.port.0;
            out[offset + 3] = 0;
        }
        Ok(needed)
    }
}

/// A state action for the caller's transport scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouterControlAction<'a> {
    /// No response or rebroadcast is required.
    None,
    /// Send I-Am-Router-To-Network on the ingress port.
    RespondIAm {
        /// Port on which the response is emitted.
        port: PortId,
        /// Networks reachable through another port.
        networks: NetworkSet<'a>,
    },
    /// Forward an unknown Who-Is query to every port except `exclude`.
    ForwardWhoIs {
        /// Port that supplied the query and must be excluded from rebroadcast.
        exclude: PortId,
        /// Specific network query, or `None` for an all-networks query.
        network: Option<NetworkNumber>,
    },
    /// Rebroadcast a learned I-Am announcement excluding its ingress port.
    RebroadcastIAm {
        /// Ingress port to exclude from the fanout.
        exclude: PortId,
        /// Validated wire network list to copy to other ports.
        networks: NetworkList<'a>,
    },
    /// Send an Initialize-Routing-Table-ACK to the ingress peer.
    SendRoutingTableAck {
        /// Port containing the requesting peer.
        port: PortId,
        /// Detached table copied into the caller's snapshot buffer.
        table: RouteSnapshot<'a>,
    },
    /// Rebroadcast a valid, owner-authorized Router-Busy announcement.
    RebroadcastBusy {
        /// Ingress port to exclude from the fanout.
        exclude: PortId,
        /// Owner-authorized network list.
        networks: NetworkList<'a>,
    },
    /// Rebroadcast a valid, owner-authorized Router-Available announcement.
    RebroadcastAvailable {
        /// Ingress port to exclude from the fanout.
        exclude: PortId,
        /// Owner-authorized network list.
        networks: NetworkList<'a>,
    },
    /// The message type is known to be a network message but is not handled by
    /// this portable router core.  The caller may forward or reject it by
    /// policy; it must not be silently treated as a successful control action.
    Unsupported {
        /// Network-message type not handled by this core.
        message_type: u8,
    },
}

/// Counts and action produced by one accepted control message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouterControlOutcome<'a> {
    /// Action that still needs transport admission by the caller.
    pub action: RouterControlAction<'a>,
    /// Number of new learned routes installed.
    pub inserted: usize,
    /// Number of learned routes refreshed.
    pub updated: usize,
    /// Number of entries ignored because they named configured direct routes.
    pub direct_conflicts: usize,
}

/// Inputs and caller-owned output buffers for one control transition.
#[derive(Debug)]
pub struct RouterControlRequest<'a> {
    /// Stable local port on which the message arrived.
    pub ingress: PortId,
    /// Immediate link-layer address of the advertising peer.
    pub peer: LinkAddress,
    /// NPDU network-message type octet.
    pub message_type: u8,
    /// Complete network-control payload, excluding the type octet.
    pub payload: &'a [u8],
    /// Caller monotonic time.
    pub now: u64,
    /// Storage for a Who-Is response list.
    pub network_out: &'a mut [NetworkNumber],
    /// Storage for a detached routing-table snapshot.
    pub snapshot_out: &'a mut [RouteEntry],
}

/// Failure while applying a validated control message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouterControlError {
    /// The wire payload failed complete validation.
    Codec(ControlCodecError),
    /// A learned route owner must include a non-empty immediate peer address.
    MissingPeerAddress,
    /// The caller's response network buffer cannot hold the response.
    ResponseBufferTooSmall {
        /// Number of network slots needed.
        needed: usize,
        /// Number of network slots supplied.
        capacity: usize,
    },
    /// A complete table would exceed the independently bounded learned table.
    LearnedTableFull {
        /// Number of new entries in the complete transaction.
        needed: usize,
        /// Number of free learned slots.
        available: usize,
    },
    /// A caller's route snapshot buffer cannot hold all entries.
    SnapshotBufferTooSmall {
        /// Number of routes to copy.
        needed: usize,
        /// Number of route slots supplied.
        capacity: usize,
    },
}

impl From<ControlCodecError> for RouterControlError {
    fn from(error: ControlCodecError) -> Self {
        Self::Codec(error)
    }
}

/// Failure while encoding a caller-buffered control response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlOutputError {
    /// Output capacity was insufficient and no bytes were written.
    BufferTooSmall {
        /// Encoded payload length required.
        needed: usize,
        /// Caller buffer capacity.
        capacity: usize,
    },
    /// A route snapshot cannot represent more than 255 entries on the wire.
    TooManyEntries {
        /// Number of routes that could not be represented by the count octet.
        count: usize,
    },
}

/// Result of admitting a pending control action to a real bounded queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlDelivery {
    /// No egress action was required.
    NotNeeded,
    /// The egress queue owns the encoded action.
    Accepted,
    /// The egress queue could not own the action; the failure remains visible.
    Rejected(AdmissionOutcome),
}

impl ControlDelivery {
    /// Map a data-link admission result without collapsing queue failures into
    /// a successful control announcement.
    pub const fn from_admission(outcome: AdmissionOutcome) -> Self {
        match outcome {
            AdmissionOutcome::AcceptedOwned => Self::Accepted,
            other => Self::Rejected(other),
        }
    }
}

/// Fixed-capacity state handler for router network-control messages.
#[derive(Clone, Debug)]
pub struct RouterControl<const DIRECT: usize, const LEARNED: usize> {
    routes: RouteTable<DIRECT, LEARNED>,
    busy_duration: u64,
    learned_age: u64,
}

impl<const DIRECT: usize, const LEARNED: usize> RouterControl<DIRECT, LEARNED> {
    /// Construct an empty control state with the documented default timers.
    pub const fn new() -> Self {
        Self {
            routes: RouteTable::new(),
            busy_duration: DEFAULT_BUSY_DURATION,
            learned_age: DEFAULT_LEARNED_AGE,
        }
    }

    /// The explicit wire-port interpretation used by this state machine.
    pub const fn port_id_semantics(&self) -> PortIdSemantics {
        PortIdSemantics::RemoteOpaque
    }

    /// Set the caller-time Router-Busy interval.
    pub const fn with_busy_duration(mut self, duration: u64) -> Self {
        self.busy_duration = duration;
        self
    }

    /// Set the caller-time learned-route lifetime.
    pub const fn with_learned_age(mut self, age: u64) -> Self {
        self.learned_age = age;
        self
    }

    /// Add or replace a configured directly connected route.
    pub fn add_direct(
        &mut self,
        network: NetworkNumber,
        port: PortId,
    ) -> Result<(), crate::RouteTableError> {
        self.routes.add_direct(network, port)
    }

    /// Read-only route access for forwarding composition.
    pub const fn routes(&self) -> &RouteTable<DIRECT, LEARNED> {
        &self.routes
    }

    /// Mutable route access for explicit platform configuration.
    pub fn routes_mut(&mut self) -> &mut RouteTable<DIRECT, LEARNED> {
        &mut self.routes
    }

    /// Remove all learned routes and return the count removed.
    pub fn clear_learned(&mut self) -> usize {
        self.routes.clear_learned()
    }

    /// Age learned routes at an exact caller-time boundary.  A route learned
    /// at `t` expires at `t + learned_age` (inclusive).
    pub fn age(&mut self, now: u64) -> usize {
        let Some(deadline) = now.checked_sub(self.learned_age) else {
            return 0;
        };
        self.routes.expire_learned(deadline)
    }

    /// Copy a detached route snapshot into the caller's buffer.
    pub fn snapshot<'a>(
        &self,
        out: &'a mut [RouteEntry],
    ) -> Result<RouteSnapshot<'a>, RouterControlError> {
        let needed = self.routes.direct_len();
        if out.len() < needed {
            return Err(RouterControlError::SnapshotBufferTooSmall {
                needed,
                capacity: out.len(),
            });
        }
        for (slot, entry) in out.iter_mut().zip(self.routes.direct_entries()) {
            *slot = entry;
        }
        Ok(RouteSnapshot {
            entries: &out[..needed],
        })
    }

    /// Handle one complete network-control message.  All decoding occurs
    /// before any route mutation.  `network_out` is used only for a Who-Is
    /// response; `snapshot_out` is used only for a routing-table ACK.
    pub fn handle<'a>(
        &mut self,
        request: RouterControlRequest<'a>,
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        let RouterControlRequest {
            ingress,
            peer,
            message_type,
            payload,
            now,
            network_out,
            snapshot_out,
        } = request;
        let control = match decode_control(message_type, payload) {
            Ok(control) => control,
            Err(ControlCodecError::UnsupportedMessageType(message_type)) => {
                return Ok(RouterControlOutcome {
                    action: RouterControlAction::Unsupported { message_type },
                    inserted: 0,
                    updated: 0,
                    direct_conflicts: 0,
                });
            }
            Err(error) => return Err(RouterControlError::Codec(error)),
        };
        match control {
            NetworkControl::WhoIsRouter { network } => {
                self.handle_who_is(ingress, network, network_out)
            }
            NetworkControl::IAmRouter { networks } => {
                self.handle_i_am(ingress, peer, networks, now)
            }
            NetworkControl::RouterBusy { networks } => {
                self.handle_busy(ingress, peer, networks, now)
            }
            NetworkControl::RouterAvailable { networks } => {
                self.handle_available(ingress, peer, networks)
            }
            NetworkControl::InitializeRoutingTable { table } => {
                self.handle_init(ingress, peer, table, now, snapshot_out)
            }
            NetworkControl::InitializeRoutingTableAck { table } => {
                let summary = self.apply_table(ingress, peer, table, now)?;
                Ok(RouterControlOutcome {
                    action: RouterControlAction::None,
                    ..summary
                })
            }
            NetworkControl::RejectMessageToNetwork { .. } => Ok(RouterControlOutcome {
                action: RouterControlAction::Unsupported { message_type },
                inserted: 0,
                updated: 0,
                direct_conflicts: 0,
            }),
        }
    }

    fn handle_who_is<'a>(
        &self,
        ingress: PortId,
        requested: Option<NetworkNumber>,
        network_out: &'a mut [NetworkNumber],
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        let mut count = 0;
        match requested {
            Some(network) => {
                let responds = self
                    .routes
                    .lookup(network)
                    .is_some_and(|entry| entry.port != ingress);
                if responds {
                    if network_out.is_empty() {
                        return Err(RouterControlError::ResponseBufferTooSmall {
                            needed: 1,
                            capacity: 0,
                        });
                    }
                    network_out[0] = network;
                    count = 1;
                } else {
                    return Ok(RouterControlOutcome {
                        action: RouterControlAction::ForwardWhoIs {
                            exclude: ingress,
                            network: requested,
                        },
                        inserted: 0,
                        updated: 0,
                        direct_conflicts: 0,
                    });
                }
            }
            None => {
                for entry in self.routes.entries() {
                    if entry.port != ingress {
                        if count == network_out.len() {
                            return Err(RouterControlError::ResponseBufferTooSmall {
                                needed: count + 1,
                                capacity: network_out.len(),
                            });
                        }
                        network_out[count] = entry.network;
                        count += 1;
                    }
                }
                if count == 0 {
                    return Ok(RouterControlOutcome {
                        action: RouterControlAction::None,
                        inserted: 0,
                        updated: 0,
                        direct_conflicts: 0,
                    });
                }
            }
        }
        Ok(RouterControlOutcome {
            action: RouterControlAction::RespondIAm {
                port: ingress,
                networks: NetworkSet {
                    values: &network_out[..count],
                },
            },
            inserted: 0,
            updated: 0,
            direct_conflicts: 0,
        })
    }

    fn handle_i_am<'a>(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        networks: NetworkList<'a>,
        now: u64,
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        if peer.is_empty() {
            return Err(RouterControlError::MissingPeerAddress);
        }
        let (inserted, updated, direct_conflicts) =
            self.apply_networks(ingress, peer, networks, now);
        let action = if inserted + updated > 0 {
            RouterControlAction::RebroadcastIAm {
                exclude: ingress,
                networks,
            }
        } else {
            RouterControlAction::None
        };
        Ok(RouterControlOutcome {
            action,
            inserted,
            updated,
            direct_conflicts,
        })
    }

    fn handle_busy<'a>(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        networks: NetworkList<'a>,
        now: u64,
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        if peer.is_empty() {
            return Err(RouterControlError::MissingPeerAddress);
        }
        let deadline = now.saturating_add(self.busy_duration);
        let changed = if networks.is_empty() {
            self.routes.mark_all_busy_from(ingress, peer, deadline) > 0
        } else {
            let mut changed = false;
            for network in networks.iter() {
                changed |= self.routes.mark_busy_from(network, ingress, peer, deadline);
            }
            changed
        };
        Ok(RouterControlOutcome {
            action: if changed {
                RouterControlAction::RebroadcastBusy {
                    exclude: ingress,
                    networks,
                }
            } else {
                RouterControlAction::None
            },
            inserted: 0,
            updated: 0,
            direct_conflicts: 0,
        })
    }

    fn handle_available<'a>(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        networks: NetworkList<'a>,
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        if peer.is_empty() {
            return Err(RouterControlError::MissingPeerAddress);
        }
        let changed = if networks.is_empty() {
            self.routes.mark_all_available_from(ingress, peer) > 0
        } else {
            let mut changed = false;
            for network in networks.iter() {
                changed |= self.routes.mark_available_from(network, ingress, peer);
            }
            changed
        };
        Ok(RouterControlOutcome {
            action: if changed {
                RouterControlAction::RebroadcastAvailable {
                    exclude: ingress,
                    networks,
                }
            } else {
                RouterControlAction::None
            },
            inserted: 0,
            updated: 0,
            direct_conflicts: 0,
        })
    }

    fn handle_init<'a>(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        table: InitializeRoutingTable<'a>,
        now: u64,
        snapshot_out: &'a mut [RouteEntry],
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        let (summary, snapshot) = match table {
            InitializeRoutingTable::Query => {
                let summary = RouterControlOutcome {
                    action: RouterControlAction::None,
                    inserted: 0,
                    updated: 0,
                    direct_conflicts: 0,
                };
                (summary, self.snapshot(snapshot_out)?)
            }
            InitializeRoutingTable::Entries(table) => {
                // A non-empty Initialize-Routing-Table request is an update;
                // the ACK is the canonical zero-entry acknowledgement.  The
                // requester's table is not echoed as if it were local state.
                let summary = self.apply_table(ingress, peer, table, now)?;
                (
                    summary,
                    RouteSnapshot {
                        entries: &snapshot_out[..0],
                    },
                )
            }
        };
        Ok(RouterControlOutcome {
            action: RouterControlAction::SendRoutingTableAck {
                port: ingress,
                table: snapshot,
            },
            ..summary
        })
    }

    fn apply_table<'a>(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        table: RoutingTable<'a>,
        now: u64,
    ) -> Result<RouterControlOutcome<'a>, RouterControlError> {
        if peer.is_empty() {
            return Err(RouterControlError::MissingPeerAddress);
        }
        // Complete preflight makes the operation transactional when a bounded
        // learned table cannot hold all genuinely new networks.  Direct-route
        // conflicts are skipped, never overwritten.
        let mut new_count = 0usize;
        for (index, entry) in table.iter().enumerate() {
            if self.routes.lookup(entry.network).is_none()
                && !seen_network(table, index, entry.network)
            {
                new_count += 1;
            }
        }
        let available = LEARNED.saturating_sub(self.routes.learned_len());
        if new_count > available {
            return Err(RouterControlError::LearnedTableFull {
                needed: new_count,
                available,
            });
        }
        let mut inserted = 0;
        let mut updated = 0;
        let mut direct_conflicts = 0;
        for entry in table.iter() {
            match self.routes.learn(entry.network, ingress, peer, now) {
                LearnOutcome::Inserted => inserted += 1,
                LearnOutcome::Updated => updated += 1,
                LearnOutcome::DirectConflict => direct_conflicts += 1,
                LearnOutcome::TableFull => unreachable!("table capacity was preflighted"),
            }
        }
        Ok(RouterControlOutcome {
            action: RouterControlAction::None,
            inserted,
            updated,
            direct_conflicts,
        })
    }

    fn apply_networks(
        &mut self,
        ingress: PortId,
        peer: LinkAddress,
        networks: NetworkList<'_>,
        now: u64,
    ) -> (usize, usize, usize) {
        let mut inserted = 0;
        let mut updated = 0;
        let mut direct_conflicts = 0;
        for network in networks.iter() {
            match self.routes.learn(network, ingress, peer, now) {
                LearnOutcome::Inserted => inserted += 1,
                LearnOutcome::Updated => updated += 1,
                LearnOutcome::DirectConflict => direct_conflicts += 1,
                LearnOutcome::TableFull => {}
            }
        }
        (inserted, updated, direct_conflicts)
    }
}

impl<const DIRECT: usize, const LEARNED: usize> Default for RouterControl<DIRECT, LEARNED> {
    fn default() -> Self {
        Self::new()
    }
}

fn seen_network(table: RoutingTable<'_>, before: usize, network: NetworkNumber) -> bool {
    table
        .iter()
        .take(before)
        .any(|entry| entry.network == network)
}

/// Return the effective state of a route at caller time without mutating it.
pub const fn route_reachability(entry: RouteEntry, now: u64) -> Reachability {
    entry.effective_reachability(now)
}
