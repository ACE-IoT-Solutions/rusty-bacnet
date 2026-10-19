//! The MS/TP monitor screen (#970): a summary line, the tab bar, and one
//! of four views: the station grid, the token ring, findings with the join
//! wizard, and error counters. Roles, warnings and the selected tab are
//! marked by text as well as colour.

use std::collections::VecDeque;
use std::time::Duration;

use bacnet_transport::mstp::monitor::{MaxMaster, MonitorStats};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::watch::sparkline;
use super::Theme;
use crate::tui::app::monitor::{MonitorState, MonitorTab, Wizard};
use crate::tui::monitor::analysis::{self, millis, Role, Severity};
use crate::tui::monitor::MonitorSample;

/// The monitor screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, monitor: &MonitorState, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" MS/TP monitor: listen only ", theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [summary, tabs, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(inner);
    let sample = monitor.shown.as_deref();
    frame.render_widget(Paragraph::new(summary_line(monitor, sample)), summary);
    frame.render_widget(Paragraph::new(tab_bar(monitor, theme)), tabs);
    let Some(sample) = sample else {
        let text = "Listening; the first reading comes within a second.";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), body);
        return;
    };
    let lines = match monitor.tab {
        MonitorTab::Stations => stations(sample, usize::from(body.width)),
        MonitorTab::Ring => ring(monitor, sample, usize::from(body.width), theme),
        MonitorTab::Findings => findings(monitor, sample, theme),
        MonitorTab::Errors => errors(monitor, sample, theme),
    };
    let wrap = matches!(monitor.tab, MonitorTab::Findings | MonitorTab::Errors);
    let paragraph = Paragraph::new(lines);
    let paragraph = if wrap {
        paragraph.wrap(Wrap { trim: false })
    } else {
        paragraph
    };
    frame.render_widget(paragraph, body);
}

fn summary_line(monitor: &MonitorState, sample: Option<&MonitorSample>) -> Line<'static> {
    let mut text = format!("{}  {} baud", monitor.port, monitor.baud);
    if let Some(sample) = sample {
        let stations = sample.stats.present_stations().count();
        text.push_str(&format!(
            "  up {} s  {stations} stations  timestamps ±{}",
            sample.elapsed.as_secs(),
            millis(sample.stats.timing.resolution)
        ));
    }
    if monitor.paused {
        text.push_str("  PAUSED");
    }
    Line::raw(text)
}

fn tab_bar(monitor: &MonitorState, theme: &Theme) -> Line<'static> {
    let findings = monitor.latest.as_deref().map_or(0, |sample| {
        analysis::findings(sample, &monitor.thresholds).len()
    });
    let mut spans = Vec::new();
    for tab in MonitorTab::ALL {
        let mut title = tab.title().to_string();
        if tab == MonitorTab::Findings && findings > 0 {
            title.push_str(&format!(" ({findings})"));
        }
        spans.push(if tab == monitor.tab {
            Span::styled(format!("[{title}]"), theme.selected())
        } else {
            Span::raw(format!(" {title} "))
        });
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("Tab: next view", theme.dim()));
    Line::from(spans)
}

/// The address map, 64 addresses a row where they fit and 32 otherwise,
/// then a row per station.
fn stations(sample: &MonitorSample, width: usize) -> Vec<Line<'static>> {
    let rows = analysis::stations(sample);
    let role = |mac: u8| rows.iter().find(|row| row.mac == mac).map(|row| row.role);
    let per: u8 = if width >= 16 + 64 { 64 } else { 32 };
    let mut lines = Vec::new();
    for (label, first, last) in [("Masters", 0u8, 127u8), ("Slaves", 128, 254)] {
        let mut start = first;
        loop {
            let end = start.saturating_add(per - 1).min(last);
            let cells: String = (start..=end)
                .map(|mac| role(mac).map_or('.', Role::letter))
                .collect();
            let label = if start == first { label } else { "" };
            lines.push(Line::raw(format!("{label:<8}{start:>3}-{end:<3} {cells}")));
            if end == last {
                break;
            }
            start = end + 1;
        }
    }
    lines.extend([
        Line::raw("M master  S slave  R router  x silent  . nothing heard"),
        Line::raw(""),
        Line::raw(format!(
            "{:>4}  {:<7} {:>8} {:>8}  {}",
            "MAC", "Role", "Device", "Frames", "Last heard"
        )),
    ]);
    let now = sample.stats.last_activity.unwrap_or_default();
    for row in &rows {
        let device = row.device.map_or_else(|| "-".into(), |d| d.to_string());
        let heard = row.last_seen.map_or_else(
            || "never".into(),
            |seen| format!("{} ago", millis(now.saturating_sub(seen))),
        );
        lines.push(Line::raw(format!(
            "{:>4}  {:<7} {device:>8} {:>8}  {heard}",
            row.mac,
            row.role.label(),
            row.frames
        )));
    }
    if rows.is_empty() {
        lines.push(Line::raw("No station heard yet."));
    }
    lines
}

fn max_master(value: Option<MaxMaster>) -> String {
    match value {
        Some(MaxMaster::Exact(v)) => v.to_string(),
        Some(MaxMaster::AtLeast(v)) => format!("≥{v}"),
        None => "?".into(),
    }
}

fn mean_count(total: u64, count: u64) -> String {
    if count == 0 {
        return "-".into();
    }
    let tenths = total * 10 / count;
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn opt(value: Option<Duration>) -> String {
    value.map_or_else(|| "-".into(), millis)
}

/// The successor chain, rotation with its sparkline, and per-master timing.
fn ring(
    monitor: &MonitorState,
    sample: &MonitorSample,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let stats: &MonitorStats = &sample.stats;
    let chain = analysis::chain(stats);
    let chain = if chain.is_empty() {
        "no token passed yet".into()
    } else {
        chain
            .iter()
            .map(|mac| format!("mstp:{mac}"))
            .collect::<Vec<_>>()
            .join(" -> ")
    };
    let r = analysis::rotation(stats);
    let over = r.max.is_some_and(|max| max > monitor.thresholds.rotation);
    let rotation = format!(
        "Rotation    min {}  avg {}  max {}  ({} samples){}",
        opt(r.min),
        opt(r.mean),
        opt(r.max),
        r.count,
        if over { "  OVER THRESHOLD" } else { "" }
    );
    let history: VecDeque<f64> = sample
        .rotations
        .iter()
        .map(|d| d.as_secs_f64() * 1000.0)
        .collect();
    let spark = sparkline(&history, width.saturating_sub(12).max(1));
    let mut lines = vec![
        Line::raw(format!("Successors  {chain}")),
        if over {
            Line::styled(rotation, theme.warn())
        } else {
            Line::raw(rotation)
        },
        Line::raw(format!("Recent      {spark}")),
        Line::styled(
            format!(
                "Thresholds  rotation {}, hold {} (:mstp rotation|hold MS)",
                millis(monitor.thresholds.rotation),
                millis(monitor.thresholds.hold)
            ),
            theme.dim(),
        ),
        Line::raw(""),
        Line::raw(format!(
            "{:>4} {:>5} {:>7} {:>15} {:>13} {:>7} {:>6}",
            "MAC", "Next", "Tokens", "Hold avg/max", "Frames/token", "Retries", "MaxMst"
        )),
    ];
    for (mac, station) in analysis::masters(stats) {
        let hold = &station.token_hold;
        let frames = &station.frames_per_token;
        let over = hold.max.is_some_and(|max| max > monitor.thresholds.hold);
        let text = format!(
            "{mac:>4} {:>5} {:>7} {:>15} {:>13} {:>7} {:>6}{}",
            station
                .next_station
                .map_or_else(|| "?".into(), |n| n.to_string()),
            station.tokens_received,
            format!("{}/{}", opt(hold.mean()), opt(hold.max)),
            format!(
                "{}/{}",
                mean_count(frames.total, frames.count),
                frames.max.map_or_else(|| "-".into(), |m| m.to_string())
            ),
            station.token_retries,
            max_master(station.max_master),
            if over { "  LONG HOLD" } else { "" }
        );
        lines.push(if over {
            Line::styled(text, theme.warn())
        } else {
            Line::raw(text)
        });
    }
    lines.push(Line::styled(
        "MaxMst: N where its polls wrap, ≥N the highest it polled, ? none seen",
        theme.dim(),
    ));
    lines
}

/// The wizard, then the findings.
fn findings(monitor: &MonitorState, sample: &MonitorSample, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match &monitor.wizard {
        None => lines.push(Line::styled(
            "J: safe join. The monitor listens, then proposes a free MAC.",
            theme.dim(),
        )),
        Some(Wizard::Listening { since }) => {
            let heard = monitor.latest.as_deref().map_or(Duration::ZERO, |latest| {
                latest.elapsed.saturating_sub(*since)
            });
            let left = monitor.listen.saturating_sub(heard).as_secs();
            lines.push(Line::styled(
                format!("Safe join: listening, {left} s to go (Esc cancels)."),
                theme.accent(),
            ));
        }
        Some(Wizard::Failed(why)) => {
            lines.push(Line::styled(format!("Safe join: {why}"), theme.warn()));
            lines.push(Line::styled("J listens again.", theme.dim()));
        }
        Some(Wizard::Proposed { proposal, selected }) => {
            let mac = proposal.candidates[*selected];
            lines.push(Line::styled(
                format!(
                    "Safe join: MAC {mac}, Max_Master {}, Max_Info_Frames {}.",
                    proposal.max_master_for(mac),
                    monitor.max_info_frames
                ),
                theme.accent(),
            ));
            let free: Vec<String> = proposal
                .candidates
                .iter()
                .map(|c| {
                    if *c == mac {
                        format!("[{c}]")
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            lines.push(Line::raw(format!("Free and polled: {}", free.join(" "))));
            for note in &proposal.notes {
                lines.push(Line::styled(format!("Note: {note}"), theme.dim()));
            }
            lines.push(Line::styled(
                if monitor.join_allowed {
                    "+/- another MAC | Enter review and join (y) | Esc cancel"
                } else {
                    "Passive (--passive): a proposal only; this session never joins."
                },
                theme.dim(),
            ));
        }
    }
    lines.push(Line::raw(""));
    let found = analysis::findings(sample, &monitor.thresholds);
    if found.is_empty() {
        lines.push(Line::raw(
            "No findings: no Max_Master leaves a station out, and no threshold was crossed.",
        ));
    }
    for finding in found {
        let (label, style) = match finding.severity {
            Severity::Warning => ("WARN", theme.warn()),
            Severity::Notice => ("NOTE", theme.dim()),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label}  "), style),
            Span::raw(finding.text),
        ]));
    }
    lines
}

/// Error counters, lost tokens and timing quality.
fn errors(monitor: &MonitorState, sample: &MonitorSample, theme: &Theme) -> Vec<Line<'static>> {
    let since = if monitor.baseline.is_some() {
        "since clear"
    } else {
        "since start"
    };
    let state = if monitor.paused { "PAUSED" } else { "live" };
    let mut lines = vec![Line::styled(
        format!("Counters ({since}, {state}): z clear, Space pause/follow"),
        theme.dim(),
    )];
    for (label, value) in monitor.errors().unwrap_or_default() {
        lines.push(Line::raw(format!("{label:<24}{value:>10}")));
    }
    lines.push(Line::raw(format!(
        "{:<24}{:>10}  (Clause 9.10, decoded only)",
        "Extended (COBS) frames", sample.stats.cobs_frames
    )));
    lines.push(Line::raw(""));
    if sample.lost.is_empty() {
        lines.push(Line::raw("No token lost."));
    }
    for lost in &sample.lost {
        let by = lost
            .regenerated_by
            .map_or_else(|| "an unreadable frame".into(), |mac| format!("mstp:{mac}"));
        lines.push(Line::styled(
            format!(
                "Token lost at {:.1} s: silent {}, then {by}",
                lost.at.as_secs_f64(),
                millis(lost.silence)
            ),
            theme.warn(),
        ));
    }
    let timing = &sample.stats.timing;
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "Timestamps ±{} (a USB adapter's latency; lower for a native UART); up to {} \
         octets per read.",
        millis(timing.resolution),
        timing.max_octets_per_read
    )));
    lines.push(Line::styled(
        "Hardware-tested on Linux; macOS and Windows are experimental, and Windows \
         timers add to the uncertainty.",
        theme.dim(),
    ));
    lines
}
