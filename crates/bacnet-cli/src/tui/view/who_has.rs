//! Who-Has results below the device table, and the Network screen's
//! Networks tab (#960). Every state is marked by text as well as colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::Frame;

use super::Theme;
use crate::tui::app::devices::age_label;
use crate::tui::app::App;

/// The most result rows the Who-Has panel takes room for.
const PANEL_ROWS: usize = 6;

/// The Devices screen: the table, and the `:find` results below it.
pub(super) fn devices(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(search) = &app.who_has else {
        super::devices::table(frame, area, app, theme);
        return;
    };
    // Borders and the header row, around at least one line.
    let rows = search.found.len().clamp(1, PANEL_ROWS) as u16 + 3;
    let [table, panel] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(rows)]).areas(area);
    super::devices::table(frame, table, app, theme);
    let found = match search.found.len() {
        1 => "1 found".to_string(),
        n => format!("{n} found"),
    };
    let title = format!(
        " Who-Has \"{}\" | {found} | sent {} ago | :find closes ",
        search.name,
        age_label(app.now, search.sent)
    );
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(theme.dim());
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    if search.found.is_empty() {
        let text = match &search.failed {
            Some(error) => Line::styled(format!("Not sent: {error}"), theme.danger()),
            None => Line::styled("No I-Have yet.", theme.dim()),
        };
        frame.render_widget(Paragraph::new(text), inner);
        return;
    }
    let rows: Vec<Row> = search
        .found
        .iter()
        .map(|row| {
            let object = format!(
                "{}:{}",
                row.object.object_type(),
                row.object.instance_number()
            );
            let network = row
                .network
                .map_or_else(|| "local".to_string(), |n| n.to_string());
            Row::new([row.device.to_string(), object, row.address.clone(), network])
        })
        .collect();
    let widths = [
        Constraint::Length(9),
        Constraint::Min(20),
        Constraint::Min(21),
        Constraint::Length(7),
    ];
    let table = Table::new(rows, widths)
        .header(Row::new(["Device", "Object", "From", "Network"]).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// The Networks tab's subtitle: routers heard, the local network's number,
/// and when the routers were last asked.
pub(super) fn networks_subtitle(app: &App) -> String {
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
pub(super) fn networks(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
        let text = "DUPLICATE: more than one router reaches this network number; \
                    two networks may share it, or the routers form a loop.";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.warn())), note);
    }
}
