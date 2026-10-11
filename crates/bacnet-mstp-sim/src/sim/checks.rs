//! The invariants checked continuously while a scenario runs.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::MasterState;

use super::master::PendingTx;
use super::{NodeKind, Sim};
use crate::line::{Driver, TxId};
use crate::report::{Collision, CollisionClass, Episode, ViolationKind, WireFrame};
use crate::trace::{Millis, TraceKind, Who};

/// States in which a master acts on the token it holds. A master in
/// PASS_TOKEN has handed it on and is only watching for the successor to use
/// it, so it is not counted.
pub(crate) fn holds_token(state: MasterState) -> bool {
    matches!(
        state,
        MasterState::UseToken
            | MasterState::DoneWithToken
            | MasterState::WaitForReply
            | MasterState::PollForMaster
    )
}

impl Sim {
    /// Every frame decodes unless a fault touched it, and, until the run is
    /// perturbed, every master that heard a clean frame whole counted it.
    pub(super) fn check_frame(&mut self, frame: &WireFrame, listeners: &[(usize, u32)]) {
        let by_master = matches!(frame.driver, Driver::Station(station)
            if matches!(self.nodes[self.index_of(station)].kind, NodeKind::Master(_)));
        if by_master && frame.is_clean() && frame.header.is_none() {
            self.violation(
                ViolationKind::Undecodable,
                format!(
                    "{:?} sent {} octets that do not decode",
                    frame.driver,
                    frame.octets.len()
                ),
            );
        }
        if self.perturbed || !frame.is_clean() || frame.header.is_none() {
            return;
        }
        let mut missed = Vec::new();
        let mut blind = Vec::new();
        for &(node, before) in listeners {
            let station = self.nodes[node].station;
            let listened_throughout = self.nodes[node]
                .listening_since
                .is_some_and(|since| since <= frame.start);
            let NodeKind::Master(master) = &self.nodes[node].kind else {
                continue;
            };
            let Some(core) = master.core.as_ref() else {
                continue;
            };
            let after = core.counters().frames_received;
            if listened_throughout && after != before + 1 {
                let text = format!("station {station} counted {}", after - before);
                if master.blind_during(frame.start, frame.end) {
                    blind.push(text);
                } else {
                    missed.push(text);
                }
            }
        }
        if !blind.is_empty() {
            self.report.blind_spot_frames += 1;
            let detail = format!(
                "clean frame {frame}: {} (slow adapter blind spot)",
                blind.join(", ")
            );
            if self.config.tolerance.slow_adapter_blind_spot {
                self.note(Who::Sim, TraceKind::Note(detail));
            } else {
                missed.push(detail);
            }
        }
        if !missed.is_empty() {
            self.violation(
                ViolationKind::ListenerMissedFrame,
                format!("clean frame {frame}: {}", missed.join(", ")),
            );
        }
    }

    /// Two drivers' octets overlap: explain it or fail.
    pub(super) fn on_overlap(&mut self, tx: TxId, driver: Driver, other: Driver) {
        let already = self
            .meta
            .get(&tx)
            .is_some_and(|meta| meta.collided_with.contains(&other));
        if already {
            return;
        }
        if let Some(meta) = self.meta.get_mut(&tx) {
            meta.collided_with.push(other);
        }
        let other_tx = match other {
            Driver::Station(station) => self.nodes[self.index_of(station)].driving,
            Driver::Noise(_) => None,
        };
        if let Some(meta) = other_tx.and_then(|other_tx| self.meta.get_mut(&other_tx)) {
            meta.collided_with.push(driver);
        }
        let class = self.classify(driver, other);
        let collision = Collision {
            at: self.now,
            driver,
            other,
            class,
        };
        self.report.collisions.push(collision);
        self.note(
            Who::Sim,
            TraceKind::Note(format!("collision {driver:?} over {other:?}: {class:?}")),
        );
        self.perturb("a collision");
        let tolerated = match class {
            CollisionClass::Noise
            | CollisionClass::GenerationRace
            | CollisionClass::SlowAdapter => true,
            CollisionClass::Scripted => self.config.tolerance.scripted_collisions,
            CollisionClass::Unexplained => false,
        };
        if !tolerated {
            self.violation(
                ViolationKind::Collision,
                format!("{driver:?} started an octet over {other:?} ({class:?})"),
            );
        }
    }

    fn classify(&self, driver: Driver, other: Driver) -> CollisionClass {
        if matches!(driver, Driver::Noise(_)) || matches!(other, Driver::Noise(_)) {
            return CollisionClass::Noise;
        }
        let master_of = |driver: Driver| match driver {
            Driver::Station(station) => match &self.nodes[self.index_of(station)].kind {
                NodeKind::Master(master) => Some(master),
                NodeKind::Scripted(_) => None,
            },
            Driver::Noise(_) => None,
        };
        let (Some(first), Some(second)) = (master_of(driver), master_of(other)) else {
            return CollisionClass::Scripted;
        };
        if first.generating && second.generating {
            CollisionClass::GenerationRace
        } else if first.spec.adapter.is_slow() || second.spec.adapter.is_slow() {
            CollisionClass::SlowAdapter
        } else {
            CollisionClass::Unexplained
        }
    }

    /// A master starts a frame no sooner than its Tturnaround after the last
    /// character on the line, its own included.
    pub(super) fn check_turnaround(&mut self, node: usize, pending: &PendingTx) {
        let now = self.now;
        let Some(last) = self.line.last_activity() else {
            return;
        };
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &self.nodes[node].kind else {
            return;
        };
        let turnaround = master.spec.config.t_turnaround;
        let slow = master.spec.adapter.start_delay > bacnet_mstp_core::Duration::ZERO;
        if now - last >= turnaround {
            return;
        }
        let detail = format!(
            "station {station} started {} us after line activity at {}, Tturnaround {} us",
            (now - last).as_micros(),
            Millis(last),
            turnaround.as_micros()
        );
        if slow && last > pending.dispatched {
            // The activity came after the core dispatched the frame; the
            // core treats octets as its own echo from then on.
            self.report.blind_spot_starts += 1;
            if self.config.tolerance.slow_adapter_blind_spot {
                self.note(Who::Sim, TraceKind::Note(format!("blind spot: {detail}")));
                return;
            }
        }
        self.violation(ViolationKind::Turnaround, detail);
    }

    /// A punctual master answers a Token or Poll For Master addressed to it
    /// within Tusage_delay, when nothing else has happened on the line since.
    pub(super) fn check_usage_delay(&mut self, node: usize, pending: &PendingTx) {
        let now = self.now;
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &self.nodes[node].kind else {
            return;
        };
        if master.spec.adapter.is_slow() {
            return;
        }
        let limit = master.spec.config.t_usage_delay;
        let Some((header, end)) = self.last_clean_frame() else {
            return;
        };
        if header.destination != station
            || !matches!(
                header.frame_type,
                FrameType::Token | FrameType::PollForMaster
            )
        {
            return;
        }
        let answering_poll = header.frame_type == FrameType::PollForMaster;
        let sends_reply = bacnet_mstp_codec::decode_frame(&pending.octets)
            .is_ok_and(|(frame, _)| frame.header.frame_type == FrameType::ReplyToPollForMaster);
        if answering_poll && !sends_reply {
            return;
        }
        if now - end > limit {
            self.violation(
                ViolationKind::UsageDelay,
                format!(
                    "station {station} answered a {:?} after {} us, Tusage_delay {} us",
                    header.frame_type,
                    (now - end).as_micros(),
                    limit.as_micros()
                ),
            );
        }
    }

    /// At most one master believes it holds the token. Masters that each
    /// generated their own token may race for up to the generation-race
    /// tolerance; after a fault, any duplicate may last up to the
    /// duplicate-token tolerance.
    pub(super) fn check_holders(&mut self) {
        let mut holders = Vec::new();
        let mut all_generating = true;
        for node in &self.nodes {
            let NodeKind::Master(master) = &node.kind else {
                continue;
            };
            if master
                .core
                .as_ref()
                .is_some_and(|core| holds_token(core.state()))
            {
                holders.push(node.station);
                all_generating &= master.generating;
            }
        }
        let now = self.now;
        if holders.len() < 2 {
            if let Some((start, race)) = self.duplicate_since.take() {
                let episode = Episode {
                    start,
                    length: now - start,
                };
                if race {
                    self.report.generation_races.push(episode);
                } else {
                    self.report.duplicate_token_episodes.push(episode);
                }
            }
            return;
        }
        let (start, race) = *self.duplicate_since.get_or_insert((now, all_generating));
        let (limit, what) = if race && all_generating {
            (self.config.tolerance.generation_race, "generated tokens")
        } else if self.perturbed {
            (
                self.config.tolerance.duplicate_token,
                "the token after a fault",
            )
        } else {
            (
                bacnet_mstp_core::Duration::ZERO,
                "the token with no fault injected",
            )
        };
        if !(race && all_generating) {
            self.duplicate_since = Some((start, false));
        }
        if now - start > limit || (!self.perturbed && !(race && all_generating)) {
            self.violation(
                ViolationKind::DuplicateToken,
                format!(
                    "stations {holders:?} have all held {what} since {}",
                    Millis(start)
                ),
            );
            self.duplicate_since = None;
        }
    }
}
