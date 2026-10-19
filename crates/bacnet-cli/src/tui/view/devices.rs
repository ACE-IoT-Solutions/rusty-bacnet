//! The Devices screen: the duplicate-instance banner and the live table.

use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::Frame;

use super::columns::{self, Column};
use super::{ellipsize, text_width, Theme, FULL_WIDTH};
use crate::tui::app::devices::{age_label, segmentation_label, SortKey};
use crate::tui::app::{App, Link, OpState};
use crate::tui::message::DeviceRow;

/// The banners (duplicate instances, the foreign-device registration), as
/// lines [`wrap_banner`] has already wrapped; the model keeps their text up
/// to date.
pub(super) fn banner(frame: &mut Frame, area: Rect, lines: &[String], theme: &Theme) {
    if lines.is_empty() {
        return;
    }
    let text: Vec<Line> = lines
        .iter()
        .map(|l| Line::styled(l.as_str(), theme.danger()))
        .collect();
    frame.render_widget(Paragraph::new(text).style(theme.danger()), area);
}

/// One banner as lines at most `width` columns wide, broken between words
/// (a word longer than a line is split) and each continuation indented one
/// space, so the banner's height is exactly the number of lines.
pub(super) fn wrap_banner(text: &str, width: usize) -> Vec<String> {
    const INDENT: &str = " ";
    let width = width.max(INDENT.len() + 1);
    let lead = &text[..text.len() - text.trim_start().len()];
    let mut lines = Vec::new();
    let mut line = lead.to_string();
    let mut fresh = true;
    for word in text.split_whitespace() {
        let len = |s: &str| s.chars().count();
        let sep = usize::from(!fresh);
        if len(&line) + sep + len(word) > width && !fresh {
            lines.push(std::mem::replace(&mut line, INDENT.to_string()));
            fresh = true;
        }
        if !fresh {
            line.push(' ');
        }
        let mut rest: Vec<char> = word.chars().collect();
        // A word too long for any line fills lines whole.
        while len(&line) + rest.len() > width {
            let take = width - len(&line);
            line.extend(rest.drain(..take));
            lines.push(std::mem::replace(&mut line, INDENT.to_string()));
        }
        line.extend(rest);
        fresh = false;
    }
    lines.push(line);
    lines
}

/// Text for the running or last Who-Is, for the table's bottom border.
fn op_status(app: &App) -> Option<String> {
    let op = app.op.as_ref()?;
    let what = format!("Who-Is {}", op.spec.describe());
    let replies = match op.replies {
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    };
    Some(match &op.state {
        OpState::Sending => format!("{what}: sending"),
        OpState::Listening { until } => {
            let left = until.saturating_duration_since(app.now);
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            format!("{what}: listening {secs}s, {replies}")
        }
        OpState::Done => format!("{what}: done, {replies}"),
        OpState::Cancelled => format!("{what}: cancelled, {replies}"),
        OpState::Failed(error) => format!("{what} failed: {error}"),
        OpState::CancelFailed(error) => format!("{what} cancel failed: {error}"),
    })
}

fn title(app: &App) -> String {
    let (key, descending) = app.devices.sort();
    let arrow = if descending { "↓" } else { "↑" };
    let shown = app.devices.visible_len();
    let total = app.devices.len();
    let count = if shown == total {
        total.to_string()
    } else {
        format!("{shown} of {total}")
    };
    format!(" Devices ({count}) | sort: {} {arrow} ", key.label())
}

/// What a column of the table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The selection marker.
    Marker,
    /// Device instance.
    Instance,
    /// Its address, or a routed device's station, as a target.
    Address,
    /// The router a routed device answered through.
    Via,
    /// Its network, or `local`.
    Network,
    /// Vendor.
    Vendor,
    /// Max APDU length accepted.
    Apdu,
    /// Segmentation supported.
    Segmentation,
    /// Age of the last I-Am.
    Seen,
}

/// The narrowest the vendor column gets before the device's other columns
/// are cut: the header.
const MIN_VENDOR: usize = "Vendor".len();

/// The columns for a table `width` columns wide, each sized to its content
/// over every visible row (not just those on screen, so scrolling doesn't
/// move them). From [`FULL_WIDTH`] the headers are spelled out. Instance and
/// address are never cut; the vendor gives way first, down to its header,
/// and the Via column appears only when the whole of it fits.
fn columns(width: u16, app: &App) -> Vec<Column<Field>> {
    let full = width >= FULL_WIDTH;
    let sorted = sorted_field(app.devices.sort().0);
    let column = |field: Field, short: &str, long: &str| {
        let mut header = if full { long } else { short }.to_string();
        if field == sorted {
            header.push('*');
        }
        Column::new(field, header)
    };
    let mut columns = vec![
        column(Field::Marker, "", ""),
        column(Field::Instance, "Instance", "Instance"),
        column(Field::Address, "Address", "Address"),
        column(Field::Network, "Net", "Network"),
        column(Field::Vendor, "Vendor", "Vendor"),
        column(Field::Apdu, "APDU", "Max APDU"),
        column(Field::Segmentation, "Seg", "Segmentation"),
        column(Field::Seen, "Seen", "Last seen"),
    ];
    let mut via = column(Field::Via, "Via", "Via");
    let mut routed = false;
    for row in app.devices.visible() {
        for column in &mut columns {
            column.fit(text_width(&cell(column.kind, row, app, false)));
        }
        if let Some(router) = &row.via {
            routed = true;
            via.fit(text_width(router));
        }
    }
    // Inside the borders.
    let room = usize::from(width.saturating_sub(2));
    columns::shrink(&mut columns, Field::Vendor, room, MIN_VENDOR);
    if routed {
        columns::insert_if_room(&mut columns, 3, via, room);
    }
    columns
}

/// One cell's text, before it is fitted to its column.
fn cell(field: Field, row: &DeviceRow, app: &App, selected: bool) -> String {
    match field {
        Field::Marker => if selected { ">" } else { " " }.to_string(),
        Field::Instance => row.instance.to_string(),
        Field::Address => row.address.clone(),
        Field::Via => row.via.clone().unwrap_or_default(),
        Field::Network => row
            .network
            .map_or_else(|| "local".to_string(), |n| n.to_string()),
        Field::Vendor => row.vendor_id.to_string(),
        Field::Apdu => row.max_apdu.to_string(),
        Field::Segmentation => segmentation_label(row.segmentation),
        Field::Seen => age_label(app.now, row.last_seen),
    }
}

/// A row's cells, each fitted to its column: a vendor too wide for its
/// column ends in `…`.
fn cells(columns: &[Column<Field>], row: &DeviceRow, app: &App, selected: bool) -> Vec<String> {
    columns
        .iter()
        .map(|column| {
            let text = cell(column.kind, row, app, selected);
            match column.kind {
                Field::Vendor => ellipsize(&text, column.width),
                _ => text,
            }
        })
        .collect()
}

pub(super) fn table(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut block = Block::bordered()
        .title(Line::styled(title(app), theme.accent()))
        .border_style(theme.dim());
    if !app.devices.filter().is_empty() {
        let filter = format!(" filter: \"{}\" ", app.devices.filter());
        block = block.title(Line::styled(filter, theme.warn()).alignment(Alignment::Right));
    }
    if let Some(status) = op_status(app) {
        block = block.title_bottom(Line::from(format!(" {status} ")));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.devices.visible_len() == 0 {
        empty(frame, inner, app, theme);
        return;
    }
    let columns = columns(area.width, app);
    // One line for the header row.
    let height = usize::from(inner.height.saturating_sub(1)).max(1);
    let offset = scroll(app, height);
    let selected = app.devices.selected_index();
    let rows: Vec<Row> = app
        .devices
        .visible_window(offset, height)
        .enumerate()
        .map(|(i, row)| {
            let is_selected = selected == Some(offset + i);
            let line = Row::new(cells(&columns, row, app, is_selected));
            if is_selected {
                line.style(theme.selected())
            } else {
                line
            }
        })
        .collect();
    let table = Table::new(rows, columns::constraints(&columns))
        .header(Row::new(columns::headers(&columns)).style(theme.accent()))
        .column_spacing(columns::SPACING);
    frame.render_widget(table, inner);
}

/// The column a sort key orders by, whose header is marked.
fn sorted_field(key: SortKey) -> Field {
    match key {
        SortKey::Instance => Field::Instance,
        SortKey::Address => Field::Address,
        SortKey::Network => Field::Network,
        SortKey::Vendor => Field::Vendor,
        SortKey::LastSeen => Field::Seen,
    }
}

/// First visible row, keeping the selection on screen. The table remembers
/// it between frames so the view only scrolls when the selection leaves it.
fn scroll(app: &App, height: usize) -> usize {
    let table = &app.devices;
    table.page.set(height);
    let mut offset = table.offset.get();
    if let Some(selected) = table.selected_index() {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    offset = offset.min(table.visible_len().saturating_sub(height));
    table.offset.set(offset);
    offset
}

fn empty(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = match (&app.link, app.devices.len()) {
        (Link::Down, _) => "Disconnected. Press p for profiles or c to connect.".into(),
        (Link::Disconnecting, _) => "Disconnecting; waiting for local resources to stop...".into(),
        (Link::Picking, _) => String::new(),
        (Link::Connecting, _) => "Connecting...".to_string(),
        (Link::Failed(error), _) => format!("Connection failed: {error}"),
        (Link::Passive, _) => {
            "Passive: no network client; nothing is transmitted. u shows the ledger.".into()
        }
        (Link::Up { .. }, 0) => "No devices yet. Press d to send a Who-Is.".to_string(),
        (Link::Up { .. }, _) => format!(
            "No device matches \"{}\". Esc clears the filter.",
            app.devices.filter()
        ),
    };
    let lines = vec![Line::raw(""), Line::from(Span::styled(text, theme.dim()))];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}
