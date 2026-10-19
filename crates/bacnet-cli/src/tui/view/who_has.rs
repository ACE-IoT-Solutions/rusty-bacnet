//! The `:find` panel below the device table (#960). Every state is
//! marked by text as well as colour.

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
