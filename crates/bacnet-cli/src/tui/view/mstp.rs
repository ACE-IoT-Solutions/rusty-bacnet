//! The MS/TP screen (#969): this station and its place in the token ring on
//! the left, the transport's counters on the right; and the join
//! confirmation. Every state is marked by text as well as colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::{centered, Theme};
use crate::tui::app::mstp::{JoinDialog, JoinOrigin, MstpState};
use crate::tui::app::App;

/// The MS/TP screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mstp = &app.mstp;
    if let (None, Some(monitor)) = (&mstp.station, app.monitor.as_deref()) {
        return super::monitor::draw(frame, area, monitor, theme);
    }
    if mstp.station.is_none() {
        let block = Block::bordered()
            .title(Line::styled(" MS/TP ", theme.accent()))
            .border_style(theme.dim());
        let text = if app.transport == "MSTP" {
            "Not on the trunk yet: confirm joining it (y), or connect again (c)."
        } else {
            "MS/TP only: start bacnet tui with --mstp PORT --mstp-mac N, or connect an \
             MS/TP profile (p), to join a trunk and see this station's token state here; \
             or with --mstp-monitor PORT to listen to one without transmitting."
        };
        let lines = Paragraph::new(Line::styled(text, theme.dim())).wrap(Wrap { trim: true });
        frame.render_widget(lines.block(block), area);
        return;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    participant(frame, left, mstp, theme);
    counters(frame, right, mstp, theme);
}

fn participant(frame: &mut Frame, area: Rect, mstp: &MstpState, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" This station ", theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let mut lines = Vec::new();
    if let Some(station) = &mstp.station {
        lines.push(Line::raw(format!("Port         {}", station.port)));
        lines.push(Line::raw(format!("Station      mstp:{}", station.mac)));
        lines.push(Line::raw(format!("Baud         {}", station.baud)));
        lines.push(Line::raw(format!("Max master   {}", station.max_master)));
        lines.push(Line::raw(format!(
            "Info frames  {}",
            station.max_info_frames
        )));
    }
    lines.push(Line::raw(""));
    match mstp.shown.and_then(|sample| sample.node) {
        None => lines.push(Line::styled("Token        not reported yet", theme.dim())),
        Some(node) => {
            lines.push(if node.holds_token {
                Line::styled("Token        HELD", theme.accent())
            } else {
                Line::raw("Token        waiting")
            });
            lines.push(Line::raw(format!("State        {}", node.state)));
            lines.push(Line::raw(format!(
                "Next station mstp:{}",
                node.next_station
            )));
            lines.push(Line::raw(format!(
                "Polling      mstp:{}",
                node.poll_station
            )));
            if node.sole_master {
                lines.push(Line::styled(
                    "SOLE MASTER: no other master answers",
                    theme.warn(),
                ));
            } else {
                lines.push(Line::raw("Sole master  no"));
            }
        }
    }
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph.block(block), area);
}

fn counters(frame: &mut Frame, area: Rect, mstp: &MstpState, theme: &Theme) {
    let since = if mstp.baseline.is_some() {
        "since clear"
    } else {
        "since joining"
    };
    let state = if mstp.paused { "PAUSED" } else { "live" };
    let title = Line::from(vec![
        Span::styled(" Counters ", theme.accent()),
        Span::styled(format!("({since}, {state}) "), theme.dim()),
    ]);
    let block = Block::bordered()
        .title(title)
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let lines: Vec<Line> = match mstp.counters() {
        None => vec![Line::styled("No reading yet.", theme.dim())],
        Some(rows) => {
            let mut lines: Vec<Line> = rows
                .into_iter()
                .map(|(label, value)| Line::raw(format!("{label:<20}{value:>12}")))
                .collect();
            lines.push(Line::styled(
                "z clear | Space pause/follow | Esc back",
                theme.dim(),
            ));
            lines
        }
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The join confirmation (T2): what joining does, the port, then `y`.
pub(super) fn join(frame: &mut Frame, area: Rect, dialog: &JoinDialog, theme: &Theme) {
    let rows = dialog
        .ports
        .len()
        .min(usize::from(area.height.saturating_sub(14)).max(1));
    let note = match &dialog.origin {
        JoinOrigin::Monitor { note } => Some(note.clone()),
        _ => None,
    };
    let extra = u16::from(note.is_some());
    let box_area = centered(area, 76, rows as u16 + 9 + extra);
    frame.render_widget(Clear, box_area);
    let s = &dialog.settings;
    let mut lines = vec![
        Line::raw(format!(
            "Join as master mstp:{}: {} baud, Max_Master {}, Max_Info_Frames {}.",
            s.mac, s.baud, s.max_master, s.max_info_frames
        )),
        Line::styled(
            "This station will take part in token passing: every station on the trunk",
            theme.warn(),
        ),
        Line::styled(
            "sees it, and a MAC another device already uses disrupts the whole ring.",
            theme.warn(),
        ),
    ];
    lines.extend(note.map(Line::raw));
    lines.extend([Line::raw(""), Line::styled("Serial port:", theme.accent())]);
    let offset = dialog.selected.saturating_sub(rows - 1);
    for (i, port) in dialog.ports.iter().enumerate().skip(offset).take(rows) {
        let text = format!("{} {port}", if i == dialog.selected { ">" } else { " " });
        lines.push(if i == dialog.selected {
            Line::styled(text, theme.selected())
        } else {
            Line::raw(text)
        });
    }
    lines.push(Line::raw(""));
    let keys = if dialog.ports.len() > 1 {
        "y join | Up/Down port | Esc cancel"
    } else {
        "y join | Esc cancel"
    };
    lines.push(Line::styled(keys, theme.dim()));
    let block = Block::bordered()
        .title(Line::styled(" Join the MS/TP trunk? ", theme.warn()))
        .border_style(theme.warn())
        .padding(Padding::horizontal(1));
    frame.render_widget(Paragraph::new(lines).block(block), box_area);
}
