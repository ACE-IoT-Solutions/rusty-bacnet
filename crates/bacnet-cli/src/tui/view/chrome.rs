//! The help overlay and the log pane.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::Frame;
use tracing::Level;

use super::{centered, Theme};
use crate::tui::app::watch::Screen;
use crate::tui::app::App;

/// A help section: a heading and its keys.
type Section = (&'static str, &'static [(&'static str, &'static str)]);

const GLOBAL: Section = (
    "Global",
    &[
        ("?  q  L", "Help; quit (guards open changes); log pane"),
        ("Ctrl-C", "Cancel the running operation; twice quits"),
        ("p  c  x", "Profiles; connect; disconnect/cancel"),
        ("W  u", "Arm or disarm writes; the session ledger"),
    ],
);

const DEVICES: Section = (
    "Devices",
    &[
        ("d  Enter", "Who-Is form; browse the selected device"),
        ("Up Down j k", "Move; PgUp PgDn Home End jump"),
        ("s  S  /", "Sort column; reverse; filter (Esc clears)"),
    ],
);

const BROWSER: Section = (
    "Browser",
    &[
        ("Tab  Enter", "Next pane; read the selected object"),
        ("/  r  a  Esc", "Filter; read again; watch the object; back"),
        ("w  n", "Write; relinquish a slot (writes armed)"),
    ],
);

const SCREENS: Section = (
    "Screens",
    &[
        ("3-7  1  x", "Watch/net/SC/MS-TP/capture; devices; unwatch"),
        (":", "watch DEV OBJ [PROP]; csv on|off; find NAME"),
        ("r  m  Tab", "Network: read BBMD/routers; survey; tab/pane"),
    ],
);

const WHO_IS: Section = (
    "Who-Is form",
    &[
        ("Tab Up Down", "Next or previous field"),
        ("Left Rt Space", "Change the scope"),
        ("Enter  Esc", "Send (again past a warning); close"),
    ],
);

/// The Capture screen's keys, shown in place of the device and form
/// sections there, so the overlay still fits 24 rows.
const CAPTURE: Section = (
    "Capture",
    &[
        ("Space  Tab", "Pause or follow; list, tree, hex"),
        ("Enter Left Rt", "To the tree; close or open a node"),
        ("/", "Filter dev: mac: net: svc: pdu: errors bcast"),
        ("e  E  z", "Export pcap (Ethernet; raw IPv4); clear"),
    ],
);

/// The `?` overlay.
pub(super) fn help(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let sections: &[Section] = if app.screen == Screen::Capture {
        &[GLOBAL, SCREENS, CAPTURE]
    } else {
        &[GLOBAL, DEVICES, BROWSER, SCREENS, WHO_IS]
    };
    let mut lines = Vec::new();
    for (section, keys) in sections {
        lines.push(Line::styled(format!(" {section}"), theme.accent()));
        for (key, what) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("   {key:<14}"), theme.warn()),
                Span::raw(*what),
            ]));
        }
    }
    lines.push(Line::styled(
        " Read-only until W arms writes; :writem sends a batch.",
        theme.dim(),
    ));
    let height = lines.len() as u16 + 2;
    let box_area = centered(area, 64, height);
    frame.render_widget(Clear, box_area);
    let block = Block::bordered()
        .title(Line::styled(" Help (? or Esc to close) ", theme.accent()))
        .border_style(theme.accent());
    frame.render_widget(Paragraph::new(lines).block(block), box_area);
}

/// The `L` pane: the newest log lines that fit.
pub(super) fn log_pane(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" Log (L to hide) ", theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let rows = usize::from(block.inner(area).height);
    let lines: Vec<Line> = app
        .log
        .tail(rows)
        .into_iter()
        .map(|line| {
            let level = match line.level {
                Level::ERROR => Span::styled("ERROR", theme.danger()),
                Level::WARN => Span::styled("WARN ", theme.warn()),
                Level::INFO => Span::raw("INFO "),
                Level::DEBUG => Span::styled("DEBUG", theme.dim()),
                _ => Span::styled("TRACE", theme.dim()),
            };
            Line::from(vec![
                Span::styled(format!("{} ", line.time), theme.dim()),
                level,
                Span::raw(format!(" {}", line.text)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
