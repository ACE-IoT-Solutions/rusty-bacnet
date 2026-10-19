//! The Network screen's Networks tab (#960): each network heard, the
//! routers that reach it, and a flag on a number more than one router
//! claims. Every state is marked by text as well as colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Row, Table};
use ratatui::Frame;

use super::Theme;
use crate::tui::app::devices::age_label;
use crate::tui::app::App;

/// The Networks tab's subtitle: routers heard, the local network's number,
/// and when the routers were last asked.
pub(super) fn subtitle(app: &App) -> String {
    let state = &app.network;
    let routers = match state.routers.router_count() {
        1 => "1 router".to_string(),
        n => format!("{n} routers"),
    };
    let local = match state.local_number {
        Some((network, true)) => format!(" | local network {network} (configured)"),
        Some((network, false)) => format!(" | local network {network} (learned)"),
        None => String::new(),
    };
    let asked = match state.routers_asked {
        Some(at) => format!("asked {} ago", age_label(app.now, at)),
        None => "r asks the routers".to_string(),
    };
    format!("| {routers}{local} | {asked} ")
}

/// The Networks tab: each network heard, the routers that reach it, and a
/// flag on a network more than one router claims.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let map = &app.network.routers;
    if map.is_empty() {
        let text = "No router heard yet. r asks every router for the networks it reaches.";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), area);
        return;
    }
    let rows: Vec<Row> = map
        .rows()
        .into_iter()
        .map(|row| {
            let flag = if row.duplicate() { "DUPLICATE" } else { "" };
            let line = Row::new([
                row.network.to_string(),
                row.routers.join(", "),
                age_label(app.now, row.heard),
                flag.to_string(),
            ]);
            if row.duplicate() {
                line.style(theme.danger())
            } else {
                line
            }
        })
        .collect();
    let widths = [
        Constraint::Length(7),
        Constraint::Min(21),
        Constraint::Length(9),
        Constraint::Length(9),
    ];
    let [table, note] = Layout::vertical([Constraint::Min(2), Constraint::Length(1)]).areas(area);
    let header = Row::new(["Network", "Router", "Heard", ""]).style(theme.accent());
    frame.render_widget(
        Table::new(rows, widths).header(header).column_spacing(1),
        table,
    );
    if !map.duplicates().is_empty() {
        let text = "DUPLICATE: several routers claim one network number: a shared \
                    number, a loop, or a moved route (r asks again).";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.warn())), note);
    }
}
