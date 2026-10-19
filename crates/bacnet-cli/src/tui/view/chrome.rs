//! Status bar, footer, help overlay and log pane.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::Frame;
use tracing::Level;

use super::{centered, Theme};
use crate::tui::app::watch::Screen;
use crate::tui::app::{App, Link, Overlay};
use crate::tui::safety::Level as Safety;

/// `bacnet | BIP 10.0.1.5:47808 | up | READ-ONLY | COV 0 FD 0 | drop 0` and
/// `? help`. `COV` and `FD` are the footprint: subscriptions held on
/// devices and foreign-device registrations held by a BBMD. An armed write
/// session turns the bar red with its countdown; `ledger n` counts changes
/// still in effect.
pub(super) fn status_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let level = app.level();
    let bar = match level {
        Safety::Armed { .. } => theme.danger(),
        _ => theme.bar(),
    };
    let (endpoint, state) = match &app.link {
        Link::Down => (app.transport.to_string(), "disconnected"),
        Link::Disconnecting => (app.transport.to_string(), "disconnecting"),
        Link::Picking => (app.transport.to_string(), "choose an interface"),
        Link::Connecting => (app.transport.to_string(), "connecting"),
        Link::Up { local } => (format!("{} {local}", app.transport), "up"),
        Link::Failed(_) => (app.transport.to_string(), "failed"),
        Link::Passive => (app.transport.to_string(), "no client"),
    };
    let level = match level {
        Safety::Passive => Span::styled("PASSIVE", theme.safe()),
        Safety::ReadOnly | Safety::Expired => Span::styled("READ-ONLY", theme.safe()),
        Safety::Armed { dangerous } => {
            let left = app.safety.armed.as_ref().map_or(0, |armed| {
                armed.until.saturating_duration_since(app.now).as_secs()
            });
            let name = if dangerous { "WRITE+DANGER" } else { "WRITE" };
            Span::styled(format!("{name} {}:{:02}", left / 60, left % 60), bar)
        }
    };
    let sep = || Span::styled(" | ", bar);
    let mut spans = vec![
        Span::styled(" bacnet", bar),
        sep(),
        Span::styled(endpoint, bar),
        sep(),
        Span::styled(state, bar),
        sep(),
        level,
        sep(),
        Span::styled(
            format!(
                "COV {} FD {}",
                app.watch.stats.subscriptions,
                u8::from(app.network.foreign.is_some())
            ),
            bar,
        ),
    ];
    if !app.safety.ledger.is_empty() {
        spans.push(sep());
        spans.push(Span::styled(
            format!("ledger {}", app.safety.ledger.len()),
            bar,
        ));
    }
    spans.push(sep());
    spans.push(Span::styled(format!("drop {}", app.dropped), bar));
    let left = Line::from(spans);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(9)]).areas(area);
    frame.render_widget(Paragraph::new(left).style(bar), left_area);
    frame.render_widget(
        Paragraph::new("? help ")
            .style(bar)
            .alignment(Alignment::Right),
        right_area,
    );
}

/// Key hints, the filter line, or a status message.
pub(super) fn footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let line = if let Overlay::Command(text) = &app.overlay {
        Line::from(vec![
            Span::styled(" :", theme.accent()),
            Span::styled(format!("{text}_"), theme.selected()),
            Span::styled("  Enter run  Esc close", theme.dim()),
        ])
    } else if app.filter_editing {
        Line::from(vec![
            Span::styled(" /", theme.accent()),
            Span::styled(format!("{}_", app.filter_text()), theme.selected()),
            Span::styled("  Enter keep  Esc clear", theme.dim()),
        ])
    } else if let Some(flash) = &app.flash {
        Line::styled(format!(" {}", flash.text), theme.warn())
    } else if let Link::Failed(error) = &app.link {
        Line::styled(format!(" {error} | p settings | c retry"), theme.danger())
    } else if app.overlay != Overlay::None {
        // The open dialog lists its own keys.
        Line::raw("")
    } else {
        let hints: &[(&str, &str)] = match app.link {
            _ if app.screen == Screen::Network => &[
                ("r", "read tables"),
                ("m", "survey BBMDs"),
                ("Tab", "BDT/FDT/matrix"),
                (":", "bbmd ADDRESS"),
                ("Esc", "back"),
                ("?", "help"),
                ("q", "quit"),
            ],
            _ if app.screen == Screen::Watch => &[
                ("x", "remove"),
                (":", "command"),
                ("Esc", "back"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Link::Up { .. } if app.browser.is_some() => &[
                ("Enter", "read"),
                ("a", "watch"),
                ("Tab", "pane"),
                ("/", "filter"),
                ("r", "refresh"),
                ("Esc", "devices"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Link::Up { .. } => &[
                ("Enter", "browse"),
                ("d", "Who-Is"),
                ("/", "filter"),
                ("s", "sort"),
                ("x", "disconnect"),
                ("?", "help"),
                ("q", "quit"),
            ],
            _ => &[
                ("c", "connect"),
                ("p", "profiles"),
                ("x", "cancel"),
                ("L", "log"),
                ("q", "quit"),
            ],
        };
        let mut spans = Vec::new();
        for (key, what) in hints {
            spans.push(Span::styled(format!(" {key}"), theme.accent()));
            spans.push(Span::styled(format!(" {what} "), theme.dim()));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}

const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?  q  L", "Help; quit (guards open changes); log pane"),
            ("Ctrl-C", "Cancel the running operation; twice quits"),
            ("p  c  x", "Profiles; connect; disconnect/cancel"),
            ("W  u", "Arm or disarm writes; the session ledger"),
        ],
    ),
    (
        "Devices",
        &[
            ("d  Enter", "Who-Is form; browse the selected device"),
            ("Up Down j k", "Move; PgUp PgDn Home End jump"),
            ("s  S  /", "Sort column; reverse; filter (Esc clears)"),
        ],
    ),
    (
        "Browser",
        &[
            ("Tab  Enter", "Next pane; read the selected object"),
            ("/  r  a  Esc", "Filter; read again; watch the object; back"),
            ("w  n", "Write; relinquish a slot (writes armed)"),
        ],
    ),
    (
        "Screens",
        &[
            ("3  4  1  x", "Watch list; network; devices; stop watching"),
            (":", "watch DEVICE OBJECT [PROPERTY]; csv on|off"),
            ("r  m  Tab", "Network: read BBMD; survey BDTs; next table"),
        ],
    ),
    (
        "Who-Is form",
        &[
            ("Tab Up Down", "Next or previous field"),
            ("Left Rt Space", "Change the scope"),
            ("Enter  Esc", "Send (again past a warning); close"),
        ],
    ),
];

/// The `?` overlay.
pub(super) fn help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let mut lines = Vec::new();
    for (section, keys) in HELP {
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
