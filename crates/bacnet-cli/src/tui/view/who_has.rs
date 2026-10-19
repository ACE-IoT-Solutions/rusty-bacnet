//! The `:find` panel below the device table (#960). Every state is
//! marked by text as well as colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::Frame;

use super::columns::{self, Column};
use super::{ellipsize, sanitize, text_width, Theme, FULL_WIDTH};
use crate::tui::app::devices::age_label;
use crate::tui::app::objects::object_label;
use crate::tui::app::App;
use crate::tui::network::discovery::FoundRow;

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
    let columns = columns(panel.width, &search.found);
    let rows: Vec<Row> = search
        .found
        .iter()
        .map(|row| {
            let cells = columns.iter().map(|column| match column.kind {
                Field::Name => ellipsize(&cell(Field::Name, row), column.width),
                kind => cell(kind, row),
            });
            Row::new(cells.collect::<Vec<_>>())
        })
        .collect();
    let table = Table::new(rows, columns::constraints(&columns))
        .header(Row::new(columns::headers(&columns)).style(theme.accent()))
        .column_spacing(columns::SPACING);
    frame.render_widget(table, inner);
}

/// What a column of the results shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The device's instance.
    Device,
    /// The object, as the browser labels it.
    Object,
    /// The object's name, from the I-Have.
    Name,
    /// The device's address, or a routed device's station.
    From,
    /// The router a routed I-Have came through.
    Via,
    /// Its network, or `local`.
    Network,
}

/// The narrowest the Name column gets: its header.
const MIN_NAME: usize = "Name".len();

/// One cell's text, before it is fitted to its column. The name is the
/// device's own text, so control characters in it become spaces.
fn cell(field: Field, row: &FoundRow) -> String {
    match field {
        Field::Device => row.device.to_string(),
        Field::Object => object_label(row.object),
        Field::Name => sanitize(&row.name),
        Field::From => row.address.clone(),
        Field::Via => row.via.clone().unwrap_or_default(),
        Field::Network => row
            .network
            .map_or_else(|| "local".to_string(), |n| n.to_string()),
    }
}

/// The columns for a panel `width` wide, sized to the results. Device,
/// object, address and network are never cut; the name takes the room
/// left, cut with `…` when it is longer, and the Via column appears only
/// when the whole of it fits beside every name in full.
fn columns(width: u16, found: &[FoundRow]) -> Vec<Column<Field>> {
    let network = if width >= FULL_WIDTH {
        "Network"
    } else {
        "Net"
    };
    let mut columns = vec![
        Column::new(Field::Device, "Device"),
        Column::new(Field::Object, "Object"),
        Column::new(Field::Name, "Name"),
        Column::new(Field::From, "From"),
        Column::new(Field::Network, network),
    ];
    let mut via = Column::new(Field::Via, "Via");
    for row in found {
        for column in &mut columns {
            column.fit(text_width(&cell(column.kind, row)));
        }
        if let Some(router) = &row.via {
            via.fit(text_width(router));
        }
    }
    // Inside the borders.
    let room = usize::from(width.saturating_sub(2));
    if found.iter().any(|row| row.via.is_some()) {
        columns::insert_if_room(&mut columns, 4, via, room);
    }
    columns::shrink(&mut columns, Field::Name, room, MIN_NAME);
    columns
}
