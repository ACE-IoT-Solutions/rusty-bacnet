//! The watch screen: one row per watched point, with its latest value,
//! where values come from, their age and a sparkline drawn with the
//! eighth-block bar symbols (never Braille, which many terminal fonts lack).

use std::collections::VecDeque;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::symbols::bar;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::Frame;

use super::{Theme, FULL_WIDTH};
use crate::tui::app::devices::age_label;
use crate::tui::app::objects::{kebab, object_label};
use crate::tui::app::watch::WatchRow;
use crate::tui::app::App;
use crate::tui::watched::{Source, METER_WINDOW_SECS};

/// The bar levels, lowest first.
const LEVELS: [&str; 8] = [
    bar::ONE_EIGHTH,
    bar::ONE_QUARTER,
    bar::THREE_EIGHTHS,
    bar::HALF,
    bar::FIVE_EIGHTHS,
    bar::THREE_QUARTERS,
    bar::SEVEN_EIGHTHS,
    bar::FULL,
];

/// The newest `width` values as bars scaled between their minimum and
/// maximum; a flat run sits at half height.
pub(crate) fn sparkline(values: &VecDeque<f64>, width: usize) -> String {
    let shown: Vec<f64> = values
        .iter()
        .skip(values.len().saturating_sub(width))
        .copied()
        .collect();
    let (low, high) = shown
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    let span = high - low;
    shown
        .iter()
        .map(|v| {
            if span <= f64::EPSILON * high.abs().max(1.0) {
                return LEVELS[3];
            }
            let level = ((v - low) / span * 7.0).round() as usize;
            LEVELS[level.min(7)]
        })
        .collect()
}

fn source_label(source: &Source) -> &'static str {
    match source {
        Source::Starting => "start",
        Source::Cov { .. } => "COV",
        Source::Polled(_) => "polled",
        Source::Failed(_) => "failed",
    }
}

fn title(app: &App) -> String {
    let stats = &app.watch.stats;
    let rate = stats.recent_requests as f64 / METER_WINDOW_SECS as f64;
    format!(
        " Watch ({}) | COV {} | requests {rate:.1}/s ",
        app.watch.rows().len(),
        stats.subscriptions
    )
}

/// Fixed columns: (header, wide width, narrow width); 0 hides it.
const COLUMNS: [(&str, u16, u16); 8] = [
    ("Device", 7, 6),
    ("Object", 16, 14),
    ("Property", 14, 0),
    ("Value", 12, 10),
    ("Units", 18, 0),
    ("Flags", 15, 15),
    ("Source", 6, 6),
    ("Age", 4, 4),
];

/// Status_Flags as text badges without brackets, to fit the column:
/// `ALM FLT OVR OOS`.
fn flags_text(flags: Option<crate::tui::point::Health>) -> String {
    flags
        .map(|f| {
            f.badges()
                .iter()
                .map(|badge| badge.trim_matches(['[', ']']))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn cells(row: &WatchRow, app: &App, wide: bool, spark: usize) -> Vec<String> {
    let value = match &row.value {
        Some(Ok(value)) => value.clone(),
        Some(Err(error)) => format!("({error})"),
        None => "-".into(),
    };
    let flags = flags_text(row.flags);
    let all = [
        row.point.device.to_string(),
        object_label(row.point.object),
        kebab(row.point.property),
        value,
        row.units.clone().unwrap_or_default(),
        flags,
        source_label(&row.source).to_string(),
        row.updated
            .map_or_else(|| "-".into(), |at| age_label(app.now, at)),
    ];
    let mut out: Vec<String> = all
        .into_iter()
        .zip(COLUMNS)
        .filter(|(_, (_, w, n))| if wide { *w > 0 } else { *n > 0 })
        .map(|(text, _)| text)
        .collect();
    out.push(sparkline(&row.history, spark));
    out
}

/// The watch screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut block = Block::bordered()
        .title(Line::styled(title(app), theme.accent()))
        .border_style(theme.dim());
    if let Some(path) = &app.watch.stats.csv {
        block = block.title(
            Line::styled(format!(" csv: {path} "), theme.warn()).alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = app.watch.rows();
    if rows.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled("No points watched.", theme.dim()),
            Line::styled(
                "In the browser, select an object and press a; or type :watch DEVICE OBJECT [PROPERTY].",
                theme.dim(),
            ),
        ];
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
        return;
    }
    let [table_area, detail] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(1)]).areas(inner);
    let wide = area.width >= FULL_WIDTH;
    let fixed: Vec<(&str, u16)> = COLUMNS
        .iter()
        .map(|(h, w, n)| (*h, if wide { *w } else { *n }))
        .filter(|(_, w)| *w > 0)
        .collect();
    let used: u16 = fixed.iter().map(|(_, w)| w + 1).sum();
    let spark = table_area.width.saturating_sub(used).max(1);
    let mut widths: Vec<Constraint> = fixed.iter().map(|(_, w)| Constraint::Length(*w)).collect();
    widths.push(Constraint::Length(spark));
    let mut header: Vec<&str> = fixed.iter().map(|(h, _)| *h).collect();
    header.push("Trend");

    let height = usize::from(table_area.height.saturating_sub(1)).max(1);
    let selected = app.watch.selected();
    let offset = selected.saturating_sub(height - 1);
    let lines: Vec<Row> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, row)| {
            let line = Row::new(cells(row, app, wide, usize::from(spark)));
            if i == selected {
                line.style(theme.selected())
            } else {
                line
            }
        })
        .collect();
    let table = Table::new(lines, widths)
        .header(Row::new(header).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, table_area);

    if let Some(row) = rows.get(selected) {
        let text = match &row.source {
            Source::Starting => "reading and subscribing".to_string(),
            Source::Cov { lifetime } => {
                format!("COV, lifetime {lifetime} s, renewed before it runs out")
            }
            Source::Polled(reason) => format!("polled: {reason}"),
            Source::Failed(error) => error.clone(),
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {}: ", object_label(row.point.object)),
                theme.accent(),
            ),
            Span::styled(text, theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(line), detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sparkline_scales_between_the_extremes_and_keeps_the_newest() {
        let values: VecDeque<f64> = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0].into();
        assert_eq!(sparkline(&values, 8), "▁▂▃▄▅▆▇█");
        let flat: VecDeque<f64> = [5.0; 4].into();
        assert_eq!(sparkline(&flat, 10), "▄▄▄▄");
        assert_eq!(sparkline(&VecDeque::new(), 5), "");
    }
}
