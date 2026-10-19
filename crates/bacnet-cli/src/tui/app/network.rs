//! The Network screen (#966): B/IP infrastructure as this client sees it.
//!
//! - The foreign-device panel: the BBMD, the time-to-live, the last
//!   BVLC-Result, a countdown to the next registration, and a banner when
//!   the BBMD refuses the registration (Annex J.5.2.2).
//! - The BDT and FDT of the target BBMD, with what each mask means and the
//!   seconds each FDT entry has left; our own FDT entry is marked, since the
//!   address the BBMD recorded is the one it forwards broadcasts to.
//! - The BDT consistency matrix of every BBMD the target's table leads to.
//!
//! The target is the BBMD the client registers with, or the one named with
//! `:bbmd ADDRESS`. Every read is a request the user starts (`r`, `m`), apart
//! from one FDT read when the registration is first accepted.

use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};

use super::watch::Screen;
use super::{App, Link, FLASH_FOR};
use crate::tui::message::Command;
use crate::tui::network::matrix::Matrix;
use crate::tui::network::{own_entry, BdtRow, FdtRow, Foreign, NetworkCommand, NetworkEvent};

/// The table below the foreign-device panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum View {
    /// The target's BDT.
    #[default]
    Bdt,
    /// The target's FDT.
    Fdt,
    /// The consistency matrix.
    Matrix,
}

/// A key on the Network screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkKey {
    /// `r`: read the target's tables again, or survey again on the matrix.
    Refresh,
    /// `m`: survey every BBMD for the matrix.
    Survey,
    /// Tab: the next table, or the previous one.
    Cycle {
        /// Go backwards.
        back: bool,
    },
}

/// A table read, with when it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fetched<T> {
    /// The BBMD read.
    pub(crate) bbmd: SocketAddrV4,
    /// When the answer arrived.
    pub(crate) at: Instant,
    /// The table, or why it could not be read.
    pub(crate) result: Result<T, String>,
}

/// The Network screen's state.
#[derive(Debug, Clone, Default)]
pub(crate) struct NetworkState {
    /// The foreign-device registration the connection asked for.
    pub(crate) foreign: Option<Foreign>,
    /// The BBMD whose tables are shown.
    pub(crate) target: Option<SocketAddrV4>,
    /// The latest registration snapshot and when it arrived.
    pub(crate) registration: Option<(ForeignRegistrationSnapshot, Instant)>,
    /// The target's BDT.
    pub(crate) bdt: Option<Fetched<Vec<BdtRow>>>,
    /// The target's FDT.
    pub(crate) fdt: Option<Fetched<Vec<FdtRow>>>,
    /// Our own entry in `fdt`, if one has our address.
    pub(crate) own: Option<usize>,
    /// Our own address, as the FDT read reported it.
    pub(crate) local: Option<SocketAddrV4>,
    /// The last survey and when it finished.
    pub(crate) matrix: Option<(Matrix, Instant)>,
    /// A table read is out.
    pub(crate) reading: bool,
    /// A survey is out.
    pub(crate) surveying: bool,
    /// The table shown.
    pub(crate) view: View,
}

impl NetworkState {
    /// The configured BBMD accepted us: start the panel on it.
    pub(crate) fn foreign_device(&mut self, foreign: Foreign) {
        self.foreign = Some(foreign);
        self.target.get_or_insert(foreign.bbmd);
    }

    /// Time until the next registration attempt at `now`.
    pub(crate) fn next_attempt(&self, now: Instant) -> Option<Duration> {
        let (snapshot, at) = self.registration.as_ref()?;
        let left = snapshot.next_attempt_in?;
        Some(left.saturating_sub(now.saturating_duration_since(*at)))
    }

    /// Time left on the lease the BBMD's last success proves, at `now`.
    pub(crate) fn lease_left(&self, now: Instant) -> Option<Duration> {
        let (snapshot, at) = self.registration.as_ref()?;
        let left = snapshot.lease_remaining.filter(|_| snapshot.registered)?;
        Some(left.saturating_sub(now.saturating_duration_since(*at)))
    }

    /// The refusal banner, while the BBMD refuses the registration.
    pub(crate) fn refusal(&self) -> Option<String> {
        let (snapshot, _) = self.registration.as_ref()?;
        if snapshot.last_outcome != Some(ForeignRegistrationOutcome::Refused) {
            return None;
        }
        let bbmd = snapshot
            .last_bbmd
            .map_or_else(|| "The BBMD".to_string(), |bbmd| format!("BBMD {bbmd}"));
        let result = snapshot
            .last_result
            .map_or_else(String::new, |code| format!(" ({code})"));
        Some(format!(
            " REFUSED {bbmd} refused the foreign-device registration{result}; \
             broadcasts through it fail until it accepts. Check its foreign-device policy."
        ))
    }

    /// Seconds an FDT entry had left at the read, less the time since.
    pub(crate) fn remaining(&self, entry: &FdtRow, now: Instant) -> u64 {
        let elapsed = self
            .fdt
            .as_ref()
            .map_or(0, |fdt| now.saturating_duration_since(fdt.at).as_secs());
        u64::from(entry.remaining).saturating_sub(elapsed)
    }
}

impl App {
    /// A Network screen report.
    pub(super) fn network_event(&mut self, event: NetworkEvent) {
        let now = self.now;
        let state = &mut self.network;
        match event {
            NetworkEvent::Registration(snapshot) => state.registration = Some((snapshot, now)),
            NetworkEvent::Tables {
                bbmd,
                local,
                bdt,
                fdt,
            } => {
                // An answer for another BBMD, such as the automatic FDT read
                // of the registration's BBMD after `:bbmd`, is not this one's.
                if state.target != Some(bbmd) {
                    return;
                }
                state.reading = false;
                state.own = fdt.as_ref().ok().and_then(|fdt| own_entry(fdt, local));
                state.local = Some(local);
                state.bdt = Some(Fetched {
                    bbmd,
                    at: now,
                    result: bdt,
                });
                state.fdt = Some(Fetched {
                    bbmd,
                    at: now,
                    result: fdt,
                });
            }
            NetworkEvent::Survey { seed, probes } => {
                // A survey from a BBMD that is no longer the target would
                // show its matrix under the new one.
                if state.target != Some(seed) {
                    return;
                }
                state.surveying = false;
                state.matrix = Some((Matrix::build(&probes), now));
            }
            NetworkEvent::Refused { command, reason } => {
                match command {
                    NetworkCommand::Tables { bbmd } if state.target == Some(bbmd) => {
                        state.reading = false;
                    }
                    NetworkCommand::Survey { seed } if state.target == Some(seed) => {
                        state.surveying = false;
                    }
                    NetworkCommand::Tables { .. } | NetworkCommand::Survey { .. } => {}
                }
                self.flash(reason, FLASH_FOR);
            }
        }
    }

    /// A key on the Network screen.
    pub(super) fn network_key(&mut self, key: NetworkKey, out: &mut Vec<Command>) {
        match key {
            NetworkKey::Cycle { back } => {
                let views = [View::Bdt, View::Fdt, View::Matrix];
                let at = views.iter().position(|v| *v == self.network.view);
                let step = if back { views.len() - 1 } else { 1 };
                self.network.view = views[(at.unwrap_or(0) + step) % views.len()];
            }
            NetworkKey::Refresh if self.network.view == View::Matrix => self.survey(out),
            NetworkKey::Refresh => self.read_tables(out),
            NetworkKey::Survey => {
                self.network.view = View::Matrix;
                self.survey(out);
            }
        }
    }

    /// `:bbmd ADDRESS`: show that BBMD's tables, and read them.
    pub(super) fn set_bbmd(&mut self, bbmd: SocketAddrV4, out: &mut Vec<Command>) {
        let state = &mut self.network;
        if state.target != Some(bbmd) {
            state.target = Some(bbmd);
            state.bdt = None;
            state.fdt = None;
            state.own = None;
            state.matrix = None;
            // Answers to the old target's requests are dropped now.
            state.reading = false;
            state.surveying = false;
        }
        self.screen = Screen::Network;
        self.read_tables(out);
    }

    /// The BBMD to read, once the session may read it.
    fn network_target(&mut self) -> Option<SocketAddrV4> {
        if self.transport != "BIP" {
            self.flash("The Network screen needs BACnet/IP.", FLASH_FOR);
            return None;
        }
        if !self.may_transmit() {
            return None;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected yet.", FLASH_FOR);
            return None;
        }
        if self.network.target.is_none() {
            self.flash(
                "No BBMD: connect with --foreign-bbmd, or type :bbmd ADDRESS.",
                FLASH_FOR,
            );
        }
        self.network.target
    }

    fn read_tables(&mut self, out: &mut Vec<Command>) {
        if let Some(bbmd) = self.network_target() {
            self.network.reading = true;
            out.push(Command::Network(NetworkCommand::Tables { bbmd }));
        }
    }

    fn survey(&mut self, out: &mut Vec<Command>) {
        if let Some(seed) = self.network_target() {
            self.network.surveying = true;
            out.push(Command::Network(NetworkCommand::Survey { seed }));
        }
    }
}
