//! What the operator can reach and press (#1895): the screens that apply
//! to the session, for the status bar's strip, and the help overlay's
//! sections, the global keys and then the current screen's.
//!
//! Each section names the keys `keymap.rs` handles on that screen; a test
//! presses every key on every screen and checks that each one the key map
//! takes is named in that screen's help.

use super::app::command_bar::COMMANDS;
use super::app::watch::Screen;
use super::app::{App, Link};

/// One screen in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tab {
    /// The key that shows it.
    pub(crate) key: char,
    /// Its name.
    pub(crate) label: &'static str,
    /// It is the screen shown.
    pub(crate) current: bool,
}

/// The screens that apply to this session, in key order: Devices,
/// Browse, Watch and Capture always; Network once a session is connected
/// (its Networks tab works on every transport), SC on BACnet/SC, MS/TP
/// with a serial port. The screen shown is listed even where it
/// doesn't apply, so the strip always says where the operator is.
pub(crate) fn screens(app: &App) -> Vec<Tab> {
    let serial = app.transport == "MSTP" || app.monitor.is_some();
    // The Networks tab (routers heard) works on every transport.
    let connected = matches!(app.link, Link::Up { .. });
    let current = match app.screen {
        Screen::Browse => '2',
        Screen::Main => '1',
        Screen::Watch => '3',
        Screen::Network => '4',
        Screen::Sc => '5',
        Screen::Mstp => '6',
        Screen::Capture => '7',
    };
    [
        ('1', "Devices", true),
        ('2', "Browse", true),
        ('3', "Watch", true),
        ('4', "Network", connected),
        ('5', "SC", app.transport == "SC"),
        ('6', "MS/TP", serial),
        ('7', "Capture", true),
    ]
    .into_iter()
    .filter(|(key, _, applies)| *applies || *key == current)
    .map(|(key, label, _)| Tab {
        key,
        label,
        current: key == current,
    })
    .collect()
}

/// A help section: its heading and its rows, each the keys (space
/// separated, as named on the keyboard) and what they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// The heading.
    pub(crate) title: &'static str,
    /// `(keys, what they do)`.
    pub(crate) rows: Vec<(String, String)>,
    /// Its keys are the screen's own (or global): false for a dialog's
    /// keys shown for reference, and for the command list.
    pub(crate) on_screen: bool,
}

impl Section {
    fn new(title: &'static str, rows: &[(&str, &str)]) -> Self {
        Self {
            title,
            on_screen: true,
            rows: rows
                .iter()
                .map(|(keys, what)| (keys.to_string(), what.to_string()))
                .collect(),
        }
    }
}

/// The movement keys, where a list takes them.
const MOVE: (&str, &str) = ("Up Down k j", "Move");
/// The keys that jump.
const JUMP: (&str, &str) = ("PgUp PgDn", "A page up or down");
/// The keys to the ends.
const ENDS: (&str, &str) = ("Home End g G", "The first or last row");

const GLOBAL: &[(&str, &str)] = &[
    ("?", "This help; Up, Down, PgUp and PgDn scroll it"),
    ("q", "Quit (asks first while writes are in effect)"),
    (
        "Ctrl-C",
        "Close a dialog or cancel the request; twice quits",
    ),
    ("L", "Show or hide the log pane"),
    (
        "W u",
        "Arm or disarm writes (read-only until then); the ledger",
    ),
    (":", "The command bar, with the commands listed below"),
    ("p c", "The profile dialog; connect again, or retry"),
    ("1 2", "Devices; 2 is Browse, the open device's browser"),
    (
        "3 4 5 6 7",
        "Watch, Network, SC, MS/TP, Capture; see the top line",
    ),
];

const DEVICES: &[(&str, &str)] = &[
    ("d", "The Who-Is form"),
    (
        "Enter",
        "Browse the selected device (the one kept open comes back as it was)",
    ),
    MOVE,
    JUMP,
    ENDS,
    (
        "/ s S",
        "Filter (Esc clears it); sort by the next column; reverse",
    ),
    ("x", "Disconnect, or cancel a connection attempt"),
    ("Esc", "Clear the filter, or a notice"),
];

const BROWSER: &[(&str, &str)] = &[
    ("Enter", "Read the selected object"),
    (
        "Tab Shift-Tab",
        "Next or previous pane; below 120 columns a commandable object's priority array is one",
    ),
    ("/ r", "Filter the objects; read the object again"),
    ("a", "Watch the selected object's present-value"),
    (
        "w n",
        "In a write session: write a property; relinquish a slot",
    ),
    MOVE,
    JUMP,
    ENDS,
    ("x", "Disconnect"),
    (
        "Esc",
        "Clear the filter, then close the device and go back (1 keeps it)",
    ),
];

const WHO_IS: &[(&str, &str)] = &[
    ("Tab Up Down", "Next or previous field"),
    ("Left Right Space", "Change the scope"),
    ("Enter Esc", "Send (again past a warning); close"),
];

const WATCH: &[(&str, &str)] = &[
    MOVE,
    JUMP,
    ENDS,
    (
        "x Delete",
        "Stop watching the row and cancel its subscription",
    ),
    ("Esc", "Back to the devices"),
];

const NETWORK: &[(&str, &str)] = &[
    (
        "Tab Shift-Tab",
        "Next or previous table: BDT, FDT, Matrix, Networks; from 160 columns, the next pane",
    ),
    (
        "r",
        "Read the BBMD's tables again; on Networks, ask the routers",
    ),
    ("m", "Survey every BBMD the tables lead to, for the matrix"),
    ("x", "Disconnect"),
    ("Esc", "Back to the devices"),
];

const SC: &[(&str, &str)] = &[
    (
        "Tab Shift-Tab",
        "Next or previous view: hub certificates, doctor, my credentials; from 160 columns the credentials view shows the hub's chain on its left",
    ),
    ("x", "Disconnect"),
    ("Esc", "Back to the devices"),
];

const MSTP: &[(&str, &str)] = &[
    ("z", "Count from now"),
    ("Space", "Pause the screen, or follow again"),
    ("x", "Disconnect, or stop the monitor"),
    (
        "Tab Shift-Tab",
        "Monitor: stations, token ring, findings, errors",
    ),
    ("J", "Monitor: start the safe join"),
    ("+ = -", "Safe join: the next or previous free MAC"),
    ("Enter", "Safe join: review the proposal in the join dialog"),
    ("Esc", "Back to the devices"),
];

const CAPTURE: &[(&str, &str)] = &[
    ("Space", "Pause the list, or follow the newest frame"),
    ("Tab Shift-Tab", "List, decode tree, hex view"),
    ("Enter Left Right", "To the tree; close or open a node"),
    ("/", "Filter: dev: mac: net: svc: pdu: errors bcast"),
    ("e E", "Export to pcap: B/IP as Ethernet, or as raw IPv4"),
    ("z", "Forget every frame"),
    ("x", "Disconnect"),
    MOVE,
    JUMP,
    ENDS,
    ("Esc", "Clear the filter, then back to the devices"),
];

/// The help for the screen shown: the global keys, the screen's own, and
/// the command bar's commands.
pub(crate) fn sections(app: &App) -> Vec<Section> {
    let mut out = vec![Section::new("Global", GLOBAL)];
    match app.screen {
        Screen::Browse => out.push(Section::new("Browse (2)", BROWSER)),
        Screen::Main => {
            out.push(Section::new("Devices (1)", DEVICES));
            out.push(Section {
                on_screen: false,
                ..Section::new("Who-Is form (d)", WHO_IS)
            });
        }
        Screen::Watch => out.push(Section::new("Watch (3)", WATCH)),
        Screen::Network => out.push(Section::new("Network (4)", NETWORK)),
        Screen::Sc => out.push(Section::new("BACnet/SC (5)", SC)),
        Screen::Mstp => out.push(Section::new("MS/TP (6)", MSTP)),
        Screen::Capture => out.push(Section::new("Capture (7)", CAPTURE)),
    }
    out.push(Section {
        title: "Commands (:)",
        on_screen: false,
        rows: COMMANDS
            .iter()
            .flat_map(|syntax| {
                syntax
                    .usage
                    .iter()
                    .map(|form| (form.to_string(), syntax.what.to_string()))
            })
            .collect(),
    });
    out
}
