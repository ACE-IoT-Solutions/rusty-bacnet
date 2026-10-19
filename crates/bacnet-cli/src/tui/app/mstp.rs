//! The MS/TP screen's state and the join confirmation (#969). No I/O.
//!
//! Joining a trunk puts a new master in the token ring, which every other
//! station on it notices, so the TUI asks first: a T2 confirmation (a
//! preview, then `y`), as for a change the safety model rates the same
//! (#964). One-shot commands take `--mstp` itself as the request.

use bacnet_transport::mstp::MstpDiagnosticsSnapshot;

use super::{App, Link, Overlay, FLASH_FOR};
use crate::mstp::MstpArgs;
use crate::tui::message::Command;
use crate::tui::mstp::{MstpReport, Sample};
use crate::tui::profile::service::Request;

/// A key on the MS/TP screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MstpKey {
    /// Count from now: later readings show what changed since.
    Clear,
    /// Stop or resume following new readings.
    ToggleWatch,
}

/// The MS/TP screen: the station joined as, the latest reading, and the
/// baseline that clear set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MstpState {
    /// The settings the session joined with.
    pub(crate) station: Option<MstpArgs>,
    /// The reading on screen.
    pub(crate) shown: Option<Sample>,
    /// The newest reading, kept while the screen is paused.
    pub(crate) latest: Option<Sample>,
    /// The counters at the last clear.
    pub(crate) baseline: Option<MstpDiagnosticsSnapshot>,
    /// New readings are not shown until watching resumes.
    pub(crate) paused: bool,
}

impl MstpState {
    /// Take a worker report.
    pub(crate) fn report(&mut self, report: MstpReport) {
        match report {
            MstpReport::Joined(station) => {
                *self = Self {
                    station: Some(station),
                    ..Self::default()
                };
            }
            MstpReport::Sample(sample) => {
                self.latest = Some(*sample);
                if !self.paused {
                    self.shown = Some(*sample);
                }
            }
        }
    }

    /// Apply a screen key.
    pub(crate) fn key(&mut self, key: MstpKey) {
        match key {
            MstpKey::Clear => self.baseline = self.latest.map(|sample| sample.counters),
            MstpKey::ToggleWatch => {
                self.paused = !self.paused;
                if !self.paused {
                    self.shown = self.latest;
                }
            }
        }
    }

    /// The counters shown: since the last clear, or since the start. A
    /// counter that saturated shows its total, which is all there is.
    pub(crate) fn counters(&self) -> Option<Vec<(&'static str, u64)>> {
        let now = self.shown?.counters;
        let base = self.baseline.unwrap_or_default();
        Some(
            rows(&now)
                .into_iter()
                .zip(rows(&base))
                .map(|((label, value), (_, before))| (label, value.saturating_sub(before)))
                .collect(),
        )
    }
}

/// The counters the screen lists, in its order.
fn rows(c: &MstpDiagnosticsSnapshot) -> [(&'static str, u64); 16] {
    let m = &c.mac;
    [
        ("Frames received", m.frames_received),
        ("Frames sent", m.frames_transmitted),
        ("NPDUs delivered", m.npdus_delivered),
        ("Tokens passed", m.token_rotations),
        ("Token retries", m.token_retries),
        ("Tokens regenerated", m.token_regenerations),
        ("Invalid frames", m.parser_errors),
        ("Frame aborts", m.frame_aborts),
        ("Reply timeouts", c.wait_for_reply_timeouts),
        ("Replies postponed", c.reply_postponed_tx),
        ("Late replies", m.late_replies),
        ("Deadline misses", m.deadline_misses),
        ("Own-MAC frames", m.own_address_frames),
        ("TX queue full", c.outbound_queue_full),
        ("Serial read errors", c.serial_read_errors),
        ("Serial write errors", c.serial_write_errors),
    ]
}

/// A key in the join confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinKey {
    /// Previous port.
    Up,
    /// Next port.
    Down,
    /// `y`: join.
    Confirm,
}

/// The join confirmation: what joining means, the port to join on, and
/// `y` to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinDialog {
    /// The settings to join with; `port` is the one first offered.
    pub(crate) settings: MstpArgs,
    /// The ports to choose from, the configured one first. A profile's
    /// port is the only one: the editor changes it.
    pub(crate) ports: Vec<String>,
    /// The port chosen.
    pub(crate) selected: usize,
    /// For a profile, the connection request to send once confirmed.
    pub(crate) then: Option<Command>,
}

impl JoinDialog {
    /// A flags-only session: `settings.port` first, then the other ports
    /// the system lists, in `mstp-ports`' order.
    pub(crate) fn for_flags(settings: MstpArgs, listed: Vec<String>) -> Self {
        let mut ports = vec![settings.port.clone()];
        ports.extend(crate::mstp::order_ports(listed));
        let ports = crate::mstp::dedup(ports);
        Self {
            settings,
            ports,
            selected: 0,
            then: None,
        }
    }

    /// A profile's connection, sent on confirmation.
    pub(crate) fn for_profile(settings: MstpArgs, then: Command) -> Self {
        Self {
            ports: vec![settings.port.clone()],
            settings,
            selected: 0,
            then: Some(then),
        }
    }

    /// The port chosen.
    pub(crate) fn port(&self) -> &str {
        &self.ports[self.selected]
    }

    /// Apply a key; on `y`, the command that joins.
    pub(crate) fn key(&mut self, key: JoinKey) -> Option<Command> {
        let last = self.ports.len().saturating_sub(1);
        match key {
            JoinKey::Up => self.selected = self.selected.saturating_sub(1),
            JoinKey::Down => self.selected = (self.selected + 1).min(last),
            JoinKey::Confirm => {
                return Some(self.then.clone().unwrap_or_else(|| Command::JoinTrunk {
                    port: self.port().to_string(),
                }));
            }
        }
        None
    }
}

impl App {
    /// Hold a profile's connection request for the join confirmation when
    /// it would join an MS/TP trunk; pass any other command through. A
    /// passive session joins nothing, so it isn't asked.
    pub(super) fn hold_for_join(&mut self, command: Command) -> Option<Command> {
        let Command::Profile(Request::Connect { .. }) = &command else {
            return Some(command);
        };
        let settings = self
            .connection
            .as_ref()
            .and_then(|connection| connection.resolved().ok())
            .and_then(|profile| profile.mstp.map(|mstp| mstp.args()));
        match settings {
            Some(settings) if !self.safety.passive => {
                self.overlay = Overlay::Join(Box::new(JoinDialog::for_profile(settings, command)));
                None
            }
            _ => Some(command),
        }
    }

    /// A key in the join confirmation: `y` joins.
    pub(super) fn join_key(&mut self, key: JoinKey, out: &mut Vec<Command>) {
        let Overlay::Join(dialog) = &mut self.overlay else {
            return;
        };
        let Some(command) = dialog.key(key) else {
            return;
        };
        let text = format!(
            "Joining the MS/TP trunk on {} as station {}.",
            dialog.port(),
            dialog.settings.mac
        );
        tracing::info!("{text}");
        self.overlay = Overlay::None;
        if let Command::Profile(Request::Connect { profile, .. }) = &command {
            let kind = profile.transport.kind();
            (self.transport, self.style) = (kind.label(), kind.style());
        }
        self.link = Link::Connecting;
        self.flash(text, FLASH_FOR);
        out.push(command);
    }

    /// Esc in the join confirmation: back to the profile, or for a
    /// flags-only session, which has nothing else to do, quit.
    pub(super) fn decline_join(&mut self) {
        let Overlay::Join(dialog) = &self.overlay else {
            return;
        };
        if dialog.then.is_some() {
            if let Some(connection) = &mut self.connection {
                connection.pending = false;
            }
            self.overlay = Overlay::Connection;
        } else {
            self.quit = true;
        }
    }
}
