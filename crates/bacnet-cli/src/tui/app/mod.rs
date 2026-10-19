//! The TEA core: one [`App`] state, one [`Action`] enum and [`update`], which
//! changes the state and returns [`Command`]s for the worker.
//!
//! `update` does no I/O: no terminal, no network, no clock. Time arrives in
//! [`Action::Tick`], so the whole state machine is testable without a
//! terminal or a transport.

pub(crate) mod browser;
pub(crate) mod capture;
pub(crate) mod command_bar;
pub(crate) mod connection;
pub(crate) mod devices;
pub(crate) mod monitor;
pub(crate) mod mstp;
pub(crate) mod network;
pub(crate) mod objects;
pub(crate) mod picker;
pub(crate) mod safety;
pub(crate) mod sc;
pub(crate) mod watch;
pub(crate) mod who_has;
pub(crate) mod whois;
mod worker_events;

use std::time::{Duration, Instant};

use self::browser::Browser;
use self::devices::{DeviceTable, Duplicates, Movement};
use self::picker::{Picker, PickerKey};
use self::watch::{Screen, WatchList};
use self::whois::{FormKey, FormOutcome, WhoIsForm};
use super::log_layer::LogRing;
use super::message::{Command, OpId, OpOutcome, WhoIsSpec, WorkerEvent};
use crate::core::target::AddressStyle;

/// A second Ctrl-C within this window quits.
pub(crate) const CTRL_C_WINDOW: Duration = Duration::from_secs(2);

/// How long a status message stays in the footer.
pub(crate) const FLASH_FOR: Duration = Duration::from_secs(4);

/// State of the link to the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Link {
    /// No client owns network resources.
    Down,
    /// Waiting for the old attempt and all its tasks to retire.
    Disconnecting,
    /// Waiting for the user to pick an interface.
    Picking,
    /// Building the client.
    Connecting,
    /// The client is up.
    Up {
        /// Local address.
        local: String,
    },
    /// Building the client failed.
    Failed(String),
    /// `--passive`: no client, nothing transmitted.
    Passive,
}

/// The modal on top of the screen, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Overlay {
    /// Named connection settings, editor and resolved preview.
    Connection,
    /// Nothing.
    None,
    /// Key help.
    Help,
    /// The Who-Is form.
    WhoIs(WhoIsForm),
    /// The interface picker.
    Picker(Picker),
    /// The `:` command bar, with what has been typed.
    Command(String),
    /// A write session dialog.
    Safety(Box<safety::Modal>),
    /// The MS/TP join confirmation.
    Join(Box<mstp::JoinDialog>),
}

/// Progress of the current operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpState {
    /// Handed to the worker.
    Sending,
    /// Sent; replies are expected until `until`.
    Listening {
        /// End of the listen window.
        until: Instant,
    },
    /// The listen window ran out.
    Done,
    /// Cancelled.
    Cancelled,
    /// Cancellation was not admitted; Ctrl-C can retry it.
    CancelFailed(String),
    /// Sending failed.
    Failed(String),
}

/// The latest Who-Is the user started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operation {
    /// Its id.
    pub(crate) id: OpId,
    /// The request.
    pub(crate) spec: WhoIsSpec,
    /// I-Am answers in range received while it ran.
    pub(crate) replies: usize,
    /// Progress.
    pub(crate) state: OpState,
}

impl Operation {
    /// True while sending or listening.
    pub(crate) fn running(&self) -> bool {
        matches!(self.state, OpState::Sending | OpState::Listening { .. })
    }
}

/// A short message in the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Flash {
    /// The text.
    pub(crate) text: String,
    /// When it disappears.
    pub(crate) until: Instant,
}

/// A key while the filter line is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterKey {
    /// Type a character.
    Char(char),
    /// Delete the last character.
    Backspace,
    /// Close the line and keep the filter.
    Commit,
    /// Close the line and clear the filter.
    Clear,
}

/// Everything that can change the state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// A frame interval elapsed.
    Tick {
        /// Current time.
        now: Instant,
        /// Worker events dropped so far.
        dropped: u64,
        /// The log ring's generation.
        log_generation: u64,
    },
    /// The terminal changed size.
    Resize,
    /// Leave the TUI.
    Quit,
    /// Ctrl-C: cancel, or quit on a second press.
    CtrlC,
    /// Esc: close, clear or dismiss.
    Escape,
    /// Show or hide the help overlay.
    ToggleHelp,
    /// Show or hide the log pane.
    ToggleLog,
    /// Open the profile manager.
    OpenConnection,
    /// A key in the connection dialog.
    Connection(connection::Key),
    /// Connect again using the selected configuration.
    Reconnect,
    /// Cancel connection or stop the active client.
    Disconnect,
    /// Open the Who-Is form.
    OpenWhoIs,
    /// Move the selection in the device table or the focused browser pane.
    Move(Movement),
    /// Enter: browse the selected device, or read the selected object.
    Open,
    /// Focus the browser's next pane, or the previous one.
    CyclePane {
        /// Go backwards.
        back: bool,
    },
    /// Read the browser's device or object again.
    Refresh,
    /// Next sort column.
    CycleSort,
    /// Flip the sort direction.
    ReverseSort,
    /// Open the filter line.
    StartFilter,
    /// A key on the filter line.
    Filter(FilterKey),
    /// A key in the Who-Is form.
    Form(FormKey),
    /// A key in the interface picker.
    Picker(PickerKey),
    /// Show the device screen or the watch list.
    ShowScreen(Screen),
    /// `a`: watch the browser's selected object.
    AddWatch,
    /// `x` on the watch list: stop watching the selected row.
    RemoveWatch,
    /// `:`: open the command bar.
    OpenCommandLine,
    /// A key on the command bar.
    CommandLine(FilterKey),
    /// `W`: arm or disarm a write session.
    ToggleArm,
    /// `u`: the session ledger.
    OpenLedger,
    /// `w` / `n` on an open object: write, or relinquish a slot.
    OpenWrite {
        /// Relinquish.
        relinquish: bool,
    },
    /// A key in a write session dialog.
    Modal(safety::ModalKey),
    /// A key on the Network screen.
    Network(network::NetworkKey),
    /// A key on the SC screen.
    Sc(sc::ScKey),
    /// A key in the MS/TP join confirmation.
    Join(mstp::JoinKey),
    /// A key on the MS/TP screen.
    Mstp(mstp::MstpKey),
    /// A key on the Capture screen.
    Capture(capture::CaptureKey),
    /// A message from the worker.
    Worker(WorkerEvent),
}

/// What [`App::new`] needs from the command line and the environment.
pub(crate) struct AppConfig {
    /// `BIP`, `BIP6` or `SC`.
    pub(crate) transport: &'static str,
    /// How the transport's addresses look.
    pub(crate) style: AddressStyle,
    /// False when `NO_COLOR` is set.
    pub(crate) color: bool,
    /// Interfaces to pick from, when BACnet/IP has no `-i`.
    pub(crate) picker: Option<Picker>,
    /// The MS/TP join confirmation, for a flags-only MS/TP session.
    pub(crate) join: Option<mstp::JoinDialog>,
    /// `--passive`.
    pub(crate) passive: bool,
    /// Start time.
    pub(crate) now: Instant,
}

/// The whole UI state.
pub(crate) struct App {
    /// `BIP`, `BIP6` or `SC`.
    pub(crate) transport: &'static str,
    /// Address style for the form's target field.
    pub(crate) style: AddressStyle,
    /// Use colour.
    pub(crate) color: bool,
    /// Link state.
    pub(crate) link: Link,
    /// Present only when profile persistence is enabled.
    pub(crate) connection: Option<connection::Connection>,
    /// The only worker generation allowed to change this session.
    pub(crate) generation: u64,
    /// The device table.
    pub(crate) devices: DeviceTable,
    /// The device browser, when open; the device table otherwise.
    pub(crate) browser: Option<Box<Browser>>,
    /// The watch list.
    pub(crate) watch: WatchList,
    /// The write session.
    pub(crate) safety: safety::SafetyState,
    /// The Network screen: foreign-device status, BDT, FDT and matrix.
    pub(crate) network: network::NetworkState,
    /// The SC screen: hub connector status, the doctor, certificates.
    pub(crate) sc: sc::ScState,
    /// The MS/TP screen: this station, its token state and counters.
    pub(crate) mstp: mstp::MstpState,
    /// A flags-only MS/TP session's join confirmation, shown again before
    /// every later join.
    pub(crate) join_again: Option<Box<mstp::JoinDialog>>,
    /// The MS/TP monitor (`--mstp-monitor`), until the session joins.
    pub(crate) monitor: Option<Box<monitor::MonitorState>>,
    /// The Capture screen: its frames outlive sessions.
    pub(crate) capture: capture::CaptureState,
    /// The `:find` search shown below the device table.
    pub(crate) who_has: Option<who_has::WhoHasSearch>,
    /// The screen in the body.
    pub(crate) screen: Screen,
    next_req: super::point::ReqId,
    /// Instances claimed by more than one address.
    pub(crate) duplicates: Duplicates,
    /// The open modal.
    pub(crate) overlay: Overlay,
    /// The filter line is open.
    pub(crate) filter_editing: bool,
    /// The log pane is shown.
    pub(crate) show_log: bool,
    /// The current or last Who-Is.
    pub(crate) op: Option<Operation>,
    last_spec: Option<WhoIsSpec>,
    next_op: OpId,
    /// Footer message.
    pub(crate) flash: Option<Flash>,
    ctrl_c_at: Option<Instant>,
    epoch: Instant,
    /// Time of the last tick.
    pub(crate) now: Instant,
    /// Worker events dropped because the UI fell behind.
    pub(crate) dropped: u64,
    /// Recent log lines.
    pub(crate) log: LogRing,
    log_generation: u64,
    /// Set when the user quits.
    pub(crate) quit: bool,
    /// Why the TUI ended, if it was an error.
    pub(crate) exit_error: Option<String>,
    /// Exit status to use with `exit_error`; `None` means 1.
    pub(crate) exit_code: Option<i32>,
    /// The screen needs a redraw.
    pub(crate) dirty: bool,
}

impl App {
    /// Initial state.
    pub(crate) fn new(config: AppConfig, log: LogRing) -> Self {
        let join_again = config.join.clone().map(Box::new);
        let (link, overlay) = match (config.picker, config.join) {
            (Some(picker), _) => (Link::Picking, Overlay::Picker(picker)),
            (None, Some(join)) => (Link::Picking, Overlay::Join(Box::new(join))),
            (None, None) => (Link::Connecting, Overlay::None),
        };
        let log_generation = log.generation();
        Self {
            transport: config.transport,
            style: config.style,
            color: config.color,
            link,
            connection: None,
            generation: 0,
            devices: DeviceTable::default(),
            browser: None,
            watch: WatchList::default(),
            safety: safety::SafetyState::new(config.passive),
            network: network::NetworkState::default(),
            sc: sc::ScState::default(),
            mstp: mstp::MstpState::default(),
            join_again,
            monitor: None,
            capture: capture::CaptureState::default(),
            who_has: None,
            screen: Screen::Main,
            next_req: 0,
            duplicates: Duplicates::default(),
            overlay,
            filter_editing: false,
            show_log: false,
            op: None,
            last_spec: None,
            next_op: 0,
            flash: None,
            ctrl_c_at: None,
            epoch: config.now,
            now: config.now,
            dropped: 0,
            log,
            log_generation,
            quit: false,
            exit_error: None,
            exit_code: None,
            dirty: true,
        }
    }

    /// Rebuild derived state (the visible table order) before drawing.
    pub(crate) fn settle(&mut self) {
        self.devices.settle();
        // Only a frame on screen is decoded.
        if self.screen == Screen::Capture {
            self.capture.settle();
        }
    }

    /// End the TUI with an error, keeping the first reason if there are two.
    pub(crate) fn fail(&mut self, error: impl Into<String>, code: i32) {
        if self.exit_error.is_none() {
            self.exit_error = Some(error.into());
            self.exit_code = Some(code);
        }
        self.quit = true;
    }

    /// True while a Who-Is is sending or listening.
    pub(crate) fn op_running(&self) -> bool {
        self.op.as_ref().is_some_and(Operation::running)
    }

    /// The filter of the list on screen: the browser's objects or the devices.
    pub(crate) fn filter_text(&self) -> &str {
        if self.screen == Screen::Capture {
            return &self.capture.filter;
        }
        match &self.browser {
            Some(browser) => browser.objects.filter(),
            None => self.devices.filter(),
        }
    }

    fn flash(&mut self, text: impl Into<String>, lasts: Duration) {
        self.flash = Some(Flash {
            text: text.into(),
            until: self.now + lasts,
        });
    }

    fn running_op_mut(&mut self, id: OpId) -> Option<&mut Operation> {
        self.op.as_mut().filter(|op| op.id == id && op.running())
    }

    fn tick(&mut self, now: Instant, dropped: u64, log_generation: u64) {
        let second = |t: Instant| t.saturating_duration_since(self.epoch).as_secs();
        // Ages and countdowns change once a second.
        let ticking = !self.devices.is_empty()
            || !self.watch.rows().is_empty()
            || matches!(self.screen, Screen::Network | Screen::Sc);
        if second(now) != second(self.now) && (ticking || self.op_running()) {
            self.dirty = true;
        }
        self.now = now;
        if dropped != self.dropped {
            self.dropped = dropped;
            self.dirty = true;
        }
        if log_generation != self.log_generation {
            self.log_generation = log_generation;
            self.dirty |= self.show_log;
        }
        if self.flash.as_ref().is_some_and(|f| now >= f.until) {
            self.flash = None;
            self.dirty = true;
        }
        if self
            .ctrl_c_at
            .is_some_and(|t| now.saturating_duration_since(t) >= CTRL_C_WINDOW)
        {
            self.ctrl_c_at = None;
        }
    }

    fn ctrl_c(&mut self, out: &mut Vec<Command>) {
        let now = self.now;
        if self
            .ctrl_c_at
            .is_some_and(|t| now.saturating_duration_since(t) < CTRL_C_WINDOW)
        {
            self.request_quit();
            return;
        }
        if self.link == Link::Picking {
            self.quit = true;
            return;
        }
        if self.link == Link::Connecting {
            self.ctrl_c_at = Some(now);
            self.disconnect(out);
            return;
        }
        self.ctrl_c_at = Some(now);
        let cancelled = if let Overlay::Safety(_) = self.overlay {
            // As Esc: a preview closed this way is recorded as declined.
            self.modal_key(safety::ModalKey::Esc, out);
            Some("Closed")
        } else if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            Some("Closed")
        } else if self.filter_editing {
            self.filter_editing = false;
            Some("Closed the filter")
        } else if let Some(op) = self
            .op
            .as_mut()
            .filter(|op| op.running() || matches!(op.state, OpState::CancelFailed(_)))
        {
            op.state = OpState::Cancelled;
            out.push(Command::Cancel { op: op.id });
            Some("Cancelled the Who-Is")
        } else {
            None
        };
        let text = match cancelled {
            Some(what) => format!("{what}. Press Ctrl-C again to quit."),
            None => "Press Ctrl-C again, or q, to quit.".to_string(),
        };
        self.flash(text, CTRL_C_WINDOW);
    }

    fn escape(&mut self, out: &mut Vec<Command>) {
        match self.overlay {
            Overlay::Picker(_) => self.quit = true,
            Overlay::Help | Overlay::WhoIs(_) | Overlay::Connection | Overlay::Command(_) => {
                self.overlay = Overlay::None
            }
            // As the dialog's own Esc: a preview closed is a decline.
            Overlay::Safety(_) => self.modal_key(safety::ModalKey::Esc, out),
            Overlay::Join(_) => self.decline_join(),
            Overlay::None if self.screen != Screen::Main => self.screen = Screen::Main,
            Overlay::None if self.browser.is_some() => match self.browser.as_deref_mut() {
                Some(browser) if !browser.objects.filter().is_empty() => {
                    browser.objects.clear_filter();
                }
                _ => self.close_browser(out),
            },
            Overlay::None if !self.devices.filter().is_empty() => {
                self.devices.clear_filter();
                self.devices.settle();
            }
            Overlay::None => self.flash = None,
        }
    }

    fn open_who_is(&mut self) {
        if !self.may_transmit() {
            return;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected yet.", FLASH_FOR);
            return;
        }
        let form = self
            .last_spec
            .as_ref()
            .map(WhoIsForm::from_spec)
            .unwrap_or_default();
        self.overlay = Overlay::WhoIs(form);
    }

    fn filter_key(&mut self, key: FilterKey) {
        if self.screen == Screen::Capture {
            return self.capture_filter_key(key);
        }
        if let Some(browser) = self.browser.as_deref_mut() {
            let objects = &mut browser.objects;
            match key {
                FilterKey::Char(ch) => objects.push_filter(ch),
                FilterKey::Backspace => objects.pop_filter(),
                FilterKey::Commit => self.filter_editing = false,
                FilterKey::Clear => {
                    objects.clear_filter();
                    self.filter_editing = false;
                }
            }
            return;
        }
        match key {
            FilterKey::Char(ch) => self.devices.push_filter(ch),
            FilterKey::Backspace => self.devices.pop_filter(),
            FilterKey::Commit => self.filter_editing = false,
            FilterKey::Clear => {
                self.devices.clear_filter();
                self.filter_editing = false;
            }
        }
        self.devices.settle();
    }

    fn form_key(&mut self, key: FormKey, out: &mut Vec<Command>) {
        let known = self.devices.len();
        let style = self.style;
        let Overlay::WhoIs(form) = &mut self.overlay else {
            return;
        };
        if let FormOutcome::Send(spec) = form.key(key, style, known) {
            self.overlay = Overlay::None;
            self.start_who_is(spec, out);
        }
    }

    fn start_who_is(&mut self, spec: WhoIsSpec, out: &mut Vec<Command>) {
        if let Some(op) = self.op.as_mut().filter(|op| op.running()) {
            // The worker ends the old listen window when the new one starts.
            op.state = OpState::Cancelled;
        }
        self.next_op += 1;
        self.last_spec = Some(spec.clone());
        self.op = Some(Operation {
            id: self.next_op,
            spec: spec.clone(),
            replies: 0,
            state: OpState::Sending,
        });
        out.push(Command::WhoIs {
            op: self.next_op,
            spec,
        });
    }

    fn picker_key(&mut self, key: PickerKey, out: &mut Vec<Command>) {
        let Overlay::Picker(picker) = &mut self.overlay else {
            return;
        };
        if let Some(iface) = picker.key(key) {
            self.overlay = Overlay::None;
            self.link = Link::Connecting;
            self.flash(
                format!(
                    "Using {} ({}, broadcast {}).",
                    iface.name, iface.ip, iface.broadcast
                ),
                FLASH_FOR,
            );
            out.push(Command::Connect {
                interface: iface.ip,
                broadcast: iface.broadcast,
            });
        }
    }

    fn reset_session(&mut self) {
        self.devices = DeviceTable::default();
        self.browser = None;
        // The worker of the ended session cancelled the subscriptions.
        self.watch = WatchList::default();
        self.network = network::NetworkState::default();
        self.sc = sc::ScState::default();
        self.mstp = mstp::MstpState::default();
        self.who_has = None;
        // Captured frames stay; the session's tap went with it.
        self.capture.sources.retain(|(_, session)| !session);
        // A new session starts read-only; its worker reports the policy
        // and the ledger again.
        self.safety.armed = None;
        self.safety.policy = None;
        self.safety.workspace = false;
        // An answer to a request the ended session had out is dropped by the
        // generation filter, so nothing would clear this later.
        self.safety.pending = None;
        self.duplicates = Duplicates::default();
        self.op = None;
        self.last_spec = None;
        self.filter_editing = false;
    }

    fn disconnect(&mut self, out: &mut Vec<Command>) {
        let monitoring = self.link == Link::Passive && self.monitor.is_some();
        if monitoring
            || matches!(
                self.link,
                Link::Connecting | Link::Up { .. } | Link::Failed(_)
            )
        {
            self.link = Link::Disconnecting;
            self.overlay = Overlay::None;
            out.push(Command::Disconnect);
        }
    }

    fn open_connection(&mut self) {
        if self.connection.is_some() {
            self.overlay = Overlay::Connection;
        } else {
            self.flash(
                "Profiles disabled. Start bacnet tui without transport flags to manage them.",
                FLASH_FOR,
            );
        }
    }

    fn connection_key(&mut self, key: connection::Key, out: &mut Vec<Command>) {
        if !matches!(self.link, Link::Down | Link::Failed(_)) {
            self.flash("Disconnect (x) before changing profiles.", FLASH_FOR);
            return;
        }
        if let Some(connection) = &mut self.connection {
            if let Some(command) = connection.key(key) {
                // Joining an MS/TP trunk is confirmed first (T2).
                let command = match self.hold_for_join(command) {
                    Some(command) => command,
                    None => return,
                };
                if let Command::Profile(super::profile::service::Request::Connect {
                    profile, ..
                }) = &command
                {
                    let kind = profile.transport.kind();
                    (self.transport, self.style) = (kind.label(), kind.style());
                    self.link = Link::Connecting;
                }
                out.push(command);
            }
        }
    }
}

/// Apply one action. Returns the commands for the worker; quitting is
/// [`App::quit`].
pub(crate) fn update(app: &mut App, action: Action) -> Vec<Command> {
    let mut out = Vec::new();
    if !matches!(action, Action::Tick { .. }) {
        app.dirty = true;
    }
    match action {
        Action::Tick {
            now,
            dropped,
            log_generation,
        } => {
            app.tick(now, dropped, log_generation);
            app.safety_tick(&mut out);
        }
        Action::Resize => {}
        Action::Quit => app.request_quit(),
        Action::CtrlC => app.ctrl_c(&mut out),
        Action::Escape => app.escape(&mut out),
        Action::ToggleHelp => {
            app.overlay = match std::mem::replace(&mut app.overlay, Overlay::None) {
                Overlay::None => Overlay::Help,
                Overlay::Help => Overlay::None,
                other => other,
            }
        }
        Action::ToggleLog => app.show_log = !app.show_log,
        Action::OpenWhoIs => app.open_who_is(),
        Action::OpenConnection => app.open_connection(),
        Action::Connection(key) => app.connection_key(key, &mut out),
        Action::Disconnect => {
            // From the connection screen, stay there to watch the link go down.
            let stay = matches!(app.overlay, Overlay::Connection);
            app.disconnect(&mut out);
            if stay {
                app.overlay = Overlay::Connection;
            }
        }
        Action::Reconnect => {
            if matches!(app.link, Link::Down | Link::Failed(_)) {
                if app.connection.is_some() {
                    app.open_connection();
                } else if let Some(join) = app.join_again.clone() {
                    // Joining again is confirmed again (T2).
                    app.overlay = Overlay::Join(join);
                } else {
                    app.link = Link::Connecting;
                    out.push(Command::Reconnect);
                }
            }
        }
        Action::Move(movement) => match app.browser.as_deref_mut() {
            _ if app.screen == Screen::Watch => app.watch.move_selection(movement),
            _ if app.screen == Screen::Capture => {
                app.capture_key(capture::CaptureKey::Move(movement));
            }
            Some(browser) => browser.move_selection(movement),
            None => app.devices.move_selection(movement),
        },
        Action::Open => app.open(&mut out),
        Action::CyclePane { back } => {
            if let Some(browser) = app.browser.as_deref_mut() {
                browser.cycle_focus(back);
            }
        }
        Action::Refresh => app.refresh(&mut out),
        Action::CycleSort => {
            app.devices.cycle_sort();
            app.devices.settle();
        }
        Action::ReverseSort => {
            app.devices.reverse();
            app.devices.settle();
        }
        Action::StartFilter => {
            if let Some(browser) = app.browser.as_deref_mut() {
                browser.focus = browser::Pane::Objects;
            }
            app.filter_editing = true;
        }
        Action::Filter(key) => app.filter_key(key),
        Action::Form(key) => app.form_key(key, &mut out),
        Action::Picker(key) => app.picker_key(key, &mut out),
        Action::ShowScreen(Screen::Capture) => app.show_capture(&mut out),
        Action::ShowScreen(screen) => app.screen = screen,
        Action::AddWatch => app.watch_selected(&mut out),
        Action::RemoveWatch => app.unwatch_selected(&mut out),
        Action::OpenCommandLine => app.open_command_line(),
        Action::CommandLine(key) => app.command_line_key(key, &mut out),
        Action::ToggleArm => app.toggle_arm(&mut out),
        Action::OpenLedger => app.open_ledger(),
        Action::OpenWrite { relinquish } => app.open_write(relinquish),
        Action::Modal(key) => app.modal_key(key, &mut out),
        Action::Network(key) => app.network_key(key, &mut out),
        Action::Sc(key) => app.sc_key(key),
        Action::Join(key) => app.join_key(key, &mut out),
        Action::Mstp(key) => app.mstp_key(key),
        Action::Capture(key) => app.capture_key(key),
        Action::Worker(event) => app.worker_event(event, &mut out),
    }
    out
}
