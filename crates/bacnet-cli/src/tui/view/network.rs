//! The Network screen: the foreign-device panel on top, and below it the
//! target BBMD's BDT or FDT, or the consistency matrix (Tab switches).
//! Every state is marked by text as well as colour.

use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, LineGauge, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use bacnet_transport::bip::ForeignRegistrationOutcome;

use super::Theme;
use crate::tui::app::devices::age_label;
use crate::tui::app::network::{Fetched, NetworkState, View};
use crate::tui::app::App;
use crate::tui::network::matrix::{Cell, Matrix};

/// Height of the foreign-device panel, borders included.
const PANEL_HEIGHT: u16 = 7;

/// Width of the BBMD address column: `255.255.255.255:65535`.
const ADDRESS: u16 = 21;

/// The Network screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let [panel, tables] =
        Layout::vertical([Constraint::Length(PANEL_HEIGHT), Constraint::Min(3)]).areas(area);
    foreign_panel(frame, panel, app, theme);
    let state = &app.network;
    let tabs = [
        (View::Bdt, "BDT"),
        (View::Fdt, "FDT"),
        (View::Matrix, "Matrix"),
    ];
    let mut title = vec![Span::styled(" ", theme.accent())];
    for (view, name) in tabs {
        let text = if view == state.view {
            format!("[{name}]")
        } else {
            name.to_string()
        };
        let style = if view == state.view {
            theme.accent()
        } else {
            theme.dim()
        };
        title.push(Span::styled(text, style));
        title.push(Span::raw(" "));
    }
    title.push(Span::styled(subtitle(app), theme.dim()));
    let block = Block::bordered()
        .title(Line::from(title))
        .border_style(theme.dim());
    let inner = block.inner(tables);
    frame.render_widget(block, tables);
    if app.transport != "BIP" {
        let text = "B/IP only: BBMDs and foreign devices belong to BACnet/IP (Annex J).";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), inner);
        return;
    }
    match state.view {
        View::Bdt => bdt(frame, inner, app, theme),
        View::Fdt => fdt(frame, inner, app, theme),
        View::Matrix => matrix(frame, inner, app, theme),
    }
}

/// Which BBMD the tables are from, and how fresh they are.
fn subtitle(app: &App) -> String {
    let state = &app.network;
    let busy = match state.view {
        View::Matrix if state.surveying => Some("surveying"),
        View::Bdt | View::Fdt if state.reading => Some("reading"),
        _ => None,
    };
    let at = match state.view {
        View::Bdt => state.bdt.as_ref().map(|f| f.at),
        View::Fdt => state.fdt.as_ref().map(|f| f.at),
        View::Matrix => state.matrix.as_ref().map(|(_, at)| *at),
    };
    let target = state
        .target
        .map_or_else(|| "no BBMD".to_string(), |t| t.to_string());
    match (busy, at) {
        (Some(busy), _) => format!("| {target} | {busy} "),
        (None, Some(at)) => format!("| {target} | read {} ago ", age_label(app.now, at)),
        (None, None) => format!("| {target} | not read: r reads, m surveys "),
    }
}

fn seconds(duration: Duration) -> u64 {
    // A countdown shows the second it is in, so 0.4 s left reads 1 s.
    duration.as_millis().div_ceil(1_000) as u64
}

fn foreign_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" Foreign device ", theme.accent()))
        .border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let state = &app.network;
    let Some(foreign) = state.foreign else {
        let lines = vec![
            Line::styled("Not registered as a foreign device.", theme.dim()),
            Line::styled(
                "Connect with --foreign-bbmd ADDRESS to register; :bbmd ADDRESS reads any BBMD's tables.",
                theme.dim(),
            ),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    };
    let [head, status, result, gauge, own] =
        Layout::vertical([Constraint::Length(1); 5]).areas(inner);
    let line = Line::from(vec![
        Span::styled("BBMD ", theme.dim()),
        Span::styled(foreign.bbmd.to_string(), theme.accent()),
        Span::styled(format!("   TTL {} s", foreign.ttl), theme.dim()),
        Span::styled(
            format!("   renews every {} s", seconds(foreign.renewal)),
            theme.dim(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), head);

    let snapshot = state.registration.as_ref().map(|(s, _)| *s);
    let refused =
        snapshot.and_then(|s| s.last_outcome) == Some(ForeignRegistrationOutcome::Refused);
    let (word, style) = match (snapshot, state.lease_left(app.now)) {
        (_, _) if refused => ("REFUSED".to_string(), theme.danger()),
        (Some(s), Some(left)) if s.registered => (
            format!(
                "registered, lease {} s left (TTL; the BBMD's FDT adds 30 s grace)",
                seconds(left)
            ),
            theme.accent(),
        ),
        (Some(_), _) => ("not registered".to_string(), theme.warn()),
        (None, _) => ("waiting for the first answer".to_string(), theme.dim()),
    };
    let counts = snapshot.map_or_else(String::new, |s| {
        format!(
            "   attempts {}  accepted {}  refused {}  lapses {}",
            s.attempts, s.accepted, s.refused, s.lapses
        )
    });
    let line = Line::from(vec![
        Span::styled("Status ", theme.dim()),
        Span::styled(word, style),
        Span::styled(counts, theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), status);

    let last = match snapshot.and_then(|s| s.last_result) {
        Some(code) => format!("{code} (0x{:04X})", code.to_raw()),
        None => "none yet".to_string(),
    };
    let outcome = snapshot
        .and_then(|s| s.last_outcome)
        .map_or_else(String::new, |o| format!(", attempt {}", outcome_label(o)));
    let line = Line::from(vec![
        Span::styled("Last BVLC-Result ", theme.dim()),
        Span::styled(
            last,
            if refused {
                theme.danger()
            } else {
                theme.accent()
            },
        ),
        Span::styled(outcome, theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), result);

    match state.next_attempt(app.now) {
        Some(left) => {
            let total = foreign.renewal.as_secs_f64().max(0.001);
            let ratio = (left.as_secs_f64() / total).clamp(0.0, 1.0);
            let widget = LineGauge::default()
                .ratio(ratio)
                .label(format!("Next registration in {:>3} s ", seconds(left)))
                .filled_symbol("█")
                .unfilled_symbol("░")
                .filled_style(theme.accent())
                .unfilled_style(theme.dim());
            frame.render_widget(widget, gauge);
        }
        None => frame.render_widget(
            Paragraph::new(Line::styled(
                "Next registration: not scheduled",
                theme.dim(),
            )),
            gauge,
        ),
    }
    frame.render_widget(Paragraph::new(own_line(state, app, theme)), own);
}

fn outcome_label(outcome: ForeignRegistrationOutcome) -> &'static str {
    match outcome {
        ForeignRegistrationOutcome::Pending => "pending",
        ForeignRegistrationOutcome::Accepted => "accepted",
        ForeignRegistrationOutcome::Refused => "refused",
        ForeignRegistrationOutcome::TimedOut => "unanswered",
        ForeignRegistrationOutcome::LocalError => "not sent",
        ForeignRegistrationOutcome::Cancelled => "cancelled",
    }
}

/// What the BBMD recorded for us, from the last FDT read.
fn own_line<'a>(state: &NetworkState, app: &App, theme: &Theme) -> Line<'a> {
    let label = Span::styled("As the BBMD sees us ", theme.dim());
    let Some(Fetched {
        result: Ok(entries),
        ..
    }) = &state.fdt
    else {
        return Line::from(vec![label, Span::styled("FDT not read yet", theme.dim())]);
    };
    match state.own.and_then(|i| entries.get(i)) {
        Some(entry) => Line::from(vec![
            label,
            Span::styled(entry.address.to_string(), theme.selected()),
            Span::styled(
                format!(
                    "  TTL {} s, {} s left",
                    entry.ttl,
                    state.remaining(entry, app.now)
                ),
                theme.dim(),
            ),
        ]),
        None => Line::from(vec![
            label,
            Span::styled(
                format!(
                    "no FDT entry has our address {}; a NAT router may have rewritten it",
                    state.local.map_or_else(String::new, |l| l.to_string())
                ),
                theme.warn(),
            ),
        ]),
    }
}

/// An error or an empty table as one line.
fn note(frame: &mut Frame, area: Rect, text: String, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Line::styled(text, theme.warn())).wrap(Wrap { trim: true }),
        area,
    );
}

fn bdt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(read) = &app.network.bdt else { return };
    let entries = match &read.result {
        Ok(entries) if entries.is_empty() => {
            return note(
                frame,
                area,
                format!("{} has an empty BDT.", read.bbmd),
                theme,
            )
        }
        Ok(entries) => entries,
        Err(error) => return note(frame, area, format!("BDT of {}: {error}", read.bbmd), theme),
    };
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let mut forwarding = entry.explain();
            if entry.address == read.bbmd {
                forwarding = format!("(this BBMD) {forwarding}");
            }
            Row::new([
                entry.address.to_string(),
                entry.mask.to_string(),
                forwarding,
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(ADDRESS),
        Constraint::Length(15),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["BBMD", "Mask", "How peers forward to it (Annex J.4.5)"])
                .style(theme.accent()),
        )
        .column_spacing(2);
    frame.render_widget(table, area);
}

fn fdt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let state = &app.network;
    let Some(read) = &state.fdt else { return };
    let entries = match &read.result {
        Ok(entries) if entries.is_empty() => {
            return note(
                frame,
                area,
                format!("{} has no foreign devices.", read.bbmd),
                theme,
            )
        }
        Ok(entries) => entries,
        Err(error) => return note(frame, area, format!("FDT of {}: {error}", read.bbmd), theme),
    };
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let ours = state.own == Some(i);
            let row = Row::new([
                entry.address.to_string(),
                format!("{} s", entry.ttl),
                format!("{} s", state.remaining(entry, app.now)),
                if ours {
                    "this client".into()
                } else {
                    String::new()
                },
            ]);
            if ours {
                row.style(theme.selected())
            } else {
                row
            }
        })
        .collect();
    let widths = [
        Constraint::Length(ADDRESS),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(Row::new(["Foreign device", "TTL", "Left", ""]).style(theme.accent()))
        .column_spacing(2);
    frame.render_widget(table, area);
}

/// A cell as text: `uni` and `dir` for the two forwarding methods, with
/// `*` when the tables disagree on the mask.
fn cell_text(cell: Cell) -> String {
    match cell {
        Cell::Listed { mask, mixed } => {
            let method = if mask == std::net::Ipv4Addr::BROADCAST {
                "uni"
            } else {
                "dir"
            };
            format!("{method}{}", if mixed { "*" } else { "" })
        }
        Cell::Absent => "-".into(),
        Cell::Missing => "MISS".into(),
        Cell::Unknown => "?".into(),
    }
}

fn matrix(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some((matrix, _)) = &app.network.matrix else {
        return note(
            frame,
            area,
            "Press m to read the BDT of every BBMD the target's table leads to.".into(),
            theme,
        );
    };
    let n = matrix.bbmds.len();
    let shown = shown(n, area.width);
    let findings = matrix.findings.len().max(1);
    let [grid, legend, list] = Layout::vertical([
        Constraint::Length(shown as u16 + 1 + u16::from(shown < n)),
        Constraint::Length(3),
        Constraint::Min(findings.min(8) as u16),
    ])
    .areas(area);
    grid_table(frame, grid, matrix, theme);
    let legend_lines = vec![
        Line::raw(""),
        Line::styled(
            "Row lists column: uni = all-ones mask, sent to the BBMD; dir = directed broadcast; \
             * = masks differ; MISS = should be listed; - = not listed; ? = not read",
            theme.dim(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(legend_lines).wrap(Wrap { trim: true }),
        legend,
    );
    let lines: Vec<Line> = if matrix.consistent() {
        vec![Line::styled(
            format!("All {n} BDTs agree: each BBMD lists itself, every entry is listed back, and masks match."),
            theme.accent(),
        )]
    } else if matrix.findings.is_empty() {
        vec![Line::styled("Some BDTs were not read.", theme.warn())]
    } else {
        matrix
            .findings
            .iter()
            .map(|finding| Line::styled(format!("! {}", finding.describe()), theme.warn()))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), list);
}

/// Width of a grid cell, and of the `+k more` column.
const CELL: u16 = 5;
const MORE: u16 = 8;
/// The most BBMDs the grid shows; the findings below still name them all.
const MAX_SHOWN: usize = 16;

/// How many of `n` BBMDs fit a grid `width` columns wide, as rows and as
/// columns, leaving room for a `+k more` column when some don't.
fn shown(n: usize, width: u16) -> usize {
    let first = ADDRESS + 4;
    let fit = |extra: u16| usize::from(width.saturating_sub(first + extra) / (CELL + 1));
    if n <= fit(0).min(MAX_SHOWN) {
        n
    } else {
        fit(MORE + 1).min(MAX_SHOWN)
    }
}

fn grid_table(frame: &mut Frame, area: Rect, matrix: &Matrix, theme: &Theme) {
    let n = matrix.bbmds.len();
    let shown = shown(n, area.width);
    let hidden = n - shown;
    let more = || Span::styled(format!("+{hidden} more"), theme.warn());
    let mut header = vec![Span::raw("BBMD (row lists column)")];
    header.extend((1..=shown).map(|i| Span::raw(format!("#{i}"))));
    if hidden > 0 {
        header.push(more());
    }
    let mut rows: Vec<Row> = matrix
        .bbmds
        .iter()
        .zip(&matrix.cells)
        .take(shown)
        .enumerate()
        .map(|(i, (bbmd, cells))| {
            let mut row = vec![Span::raw(format!("#{} {bbmd}", i + 1))];
            row.extend(cells.iter().take(shown).map(|cell| {
                let style = match cell {
                    Cell::Missing | Cell::Listed { mixed: true, .. } => theme.danger(),
                    Cell::Unknown => theme.warn(),
                    Cell::Absent => theme.dim(),
                    Cell::Listed { .. } => theme.accent(),
                };
                Span::styled(cell_text(*cell), style)
            }));
            Row::new(row)
        })
        .collect();
    if hidden > 0 {
        rows.push(Row::new([more()]));
    }
    let mut widths = vec![Constraint::Length(ADDRESS + 4)];
    widths.extend(std::iter::repeat_n(Constraint::Length(CELL), shown));
    widths.push(Constraint::Length(MORE));
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_read_as_text() {
        let unicast = std::net::Ipv4Addr::BROADCAST;
        let subnet = std::net::Ipv4Addr::new(255, 255, 255, 0);
        let listed = |mask, mixed| Cell::Listed { mask, mixed };
        assert_eq!(cell_text(listed(unicast, false)), "uni");
        assert_eq!(cell_text(listed(subnet, true)), "dir*");
        assert_eq!(cell_text(Cell::Missing), "MISS");
        assert_eq!(cell_text(Cell::Unknown), "?");
        assert_eq!(seconds(Duration::from_millis(400)), 1);
        assert_eq!(seconds(Duration::ZERO), 0);
    }

    #[test]
    fn the_grid_shows_what_fits_and_counts_the_rest() {
        // 118 columns inside the border: all of 12, 14 of 20.
        assert_eq!(shown(12, 118), 12);
        assert_eq!(shown(15, 118), 15);
        assert_eq!(shown(20, 118), 14);
        assert_eq!(shown(40, 400), MAX_SHOWN);
        assert_eq!(shown(3, 78), 3);
    }
}
