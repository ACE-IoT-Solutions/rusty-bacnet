//! Profile editor and resolved connection preview at the supported terminal sizes.

use super::{centered, Theme};
use crate::tui::app::connection::{Connection, Screen, LABELS};
use ratatui::{
    layout::Rect,
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Wrap},
    Frame,
};

pub(super) fn draw(frame: &mut Frame, area: Rect, connection: &Connection, theme: &Theme) {
    let box_area = centered(area, area.width.saturating_sub(4).min(96), 22);
    frame.render_widget(Clear, box_area);
    let mut lines = match connection.screen {
        Screen::List => list(connection, theme),
        Screen::Edit => editor(connection, theme),
        Screen::Preview => preview(connection, theme),
    };
    if connection.pending {
        lines.push(Line::styled("Working…", theme.warn()));
    }
    if let Some(error) = &connection.error {
        lines.push(Line::styled(error.clone(), theme.danger()));
    }
    let block = Block::bordered()
        .title(Line::styled(" B/IP connection profiles ", theme.accent()))
        .title_bottom(Line::styled(" Esc close | READ-ONLY ", theme.dim()))
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        box_area,
    );
}

fn list<'a>(connection: &'a Connection, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::raw("Choose a profile to review before connecting."),
        Line::raw(""),
    ];
    if connection.names.is_empty() {
        lines.push(Line::raw("No profiles. Press n to create one."));
    }
    let offset = connection.selected.saturating_sub(8);
    for (i, name) in connection.names.iter().enumerate().skip(offset).take(9) {
        let selected = i == connection.selected;
        lines.push(Line::styled(
            format!("{} {name}", if selected { ">" } else { " " }),
            if selected {
                theme.selected()
            } else {
                theme.dim()
            },
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw(
        "Enter preview | Up/Down choose | n new | r reload list",
    ));
    lines.push(Line::styled(
        format!("Config: {}", connection.root),
        theme.dim(),
    ));
    lines
}

fn editor<'a>(connection: &'a Connection, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::raw(if connection.creating {
        "Create a profile; Enter saves it without connecting."
    } else {
        "Edit profile; Enter saves it without connecting."
    })];
    for (i, label) in LABELS.iter().enumerate() {
        let focused = i == connection.focus;
        let field = &connection.fields[i];
        let tail: String = field
            .chars()
            .rev()
            .take(42)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let text = format!(
            "{} {label:<13} {tail}{}",
            if focused { ">" } else { " " },
            if focused { "_" } else { "" }
        );
        lines.push(Line::styled(
            text,
            if focused {
                theme.selected()
            } else {
                theme.dim()
            },
        ));
    }
    lines.push(Line::raw(
        "Tab/Shift-Tab fields | Backspace erase | Enter save",
    ));
    lines.push(Line::raw(
        "Blank BBMD = Normal; otherwise Foreign Device (IP[:port]).",
    ));
    lines.push(Line::raw(
        "Blank renewal = half TTL; interface 0.0.0.0 = all interfaces.",
    ));
    lines.push(Line::raw(
        "Writes: never or session. Dangerous: false. UI stays read-only.",
    ));
    lines
}

fn preview<'a>(connection: &'a Connection, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::styled(
        format!("{} — resolved connection", connection.name),
        theme.accent(),
    )];
    match connection.resolved() {
        Err(error) => lines.push(Line::styled(error, theme.danger())),
        Ok(profile) => {
            let b = &profile.bip;
            lines.push(Line::raw(format!(
                "Mode: {}",
                if b.bbmd.is_some() {
                    "Foreign Device"
                } else {
                    "Normal"
                }
            )));
            lines.push(Line::raw(format!(
                "Bind: {}:{}   Broadcast: {}",
                b.interface, b.port, b.broadcast
            )));
            lines.push(Line::raw(format!(
                "APDU timeout: {} ms   Pacing: {} ms",
                b.timeout_ms, b.min_interval_ms
            )));
            if let Some(bbmd) = b.bbmd {
                lines.push(Line::raw(format!("BBMD: {bbmd}   TTL: {} s", b.ttl)));
                lines.push(Line::raw(format!(
                    "Renewal: {}",
                    b.renewal_secs
                        .map_or_else(|| "half TTL".into(), |n| format!("{n} s"))
                )));
                lines.push(Line::styled(
                    "Local connection does not prove BBMD acceptance; see L log.",
                    theme.dim(),
                ));
            }
            lines.push(Line::raw(format!(
                "Saved writes policy: {:?}; dangerous: false. UI is read-only.",
                profile.safety.writes
            )));
            if !connection.overrides.is_empty() {
                lines.push(Line::styled(
                    "Explicit CLI overrides are included above; saved values stay separate.",
                    theme.warn(),
                ));
            }
            lines.push(Line::raw(""));
            match profile.authorized_at_unix {
                Some(at) => {
                    lines.push(Line::raw(format!("Authorization saved at Unix time {at}.")));
                    lines.push(Line::styled("Enter / c connect", theme.accent()));
                }
                None => {
                    lines.push(Line::styled(
                        "First use requires your explicit authorization:",
                        theme.warn(),
                    ));
                    lines.push(Line::raw("I am authorized to use BACnet on this network."));
                    lines.push(Line::styled(
                        "a attest, save authorization and connect | Esc cancel",
                        theme.warn(),
                    ));
                }
            }
        }
    }
    lines.push(Line::raw(
        "e edit | r reload from disk | p profiles | n new",
    ));
    lines
}
