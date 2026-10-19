//! The SC screen: the hub connector and identity panels on top, the hub's
//! certificate chain or the connection doctor in the middle (Tab switches),
//! and the event timeline below. Every state is marked by text as well as
//! colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use bacnet_transport::sc::ScConnectionState;

use super::Theme;
use crate::tui::app::devices::age_label;
use crate::tui::app::sc::{ScState, Severity, View};
use crate::tui::app::App;
use crate::tui::sc::doctor::{Diagnosis, Mark, CONNECTED, PHASES};
use crate::tui::sc::{hub_name, uuid_text, vmac_text, CertSummary};

/// Height of the status and identity panels, borders included.
const PANEL_HEIGHT: u16 = 8;
/// Height of the chain or doctor block, borders included.
const MIDDLE_HEIGHT: u16 = 15;

/// The SC screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.transport != "SC" {
        let block = Block::bordered()
            .title(Line::styled(" BACnet/SC ", theme.accent()))
            .border_style(theme.dim());
        let text = "SC only: the hub connector belongs to BACnet/SC (Annex AB). Connect an SC \
                    profile (p) to use this screen.";
        let lines = Paragraph::new(Line::styled(text, theme.dim())).wrap(Wrap { trim: true });
        frame.render_widget(lines.block(block), area);
        return;
    }
    let [top, middle, events] = Layout::vertical([
        Constraint::Length(PANEL_HEIGHT),
        Constraint::Length(MIDDLE_HEIGHT),
        Constraint::Min(3),
    ])
    .areas(area);
    let [status, identity] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(top);
    status_panel(frame, status, app, theme);
    identity_panel(frame, identity, app, theme);
    let sc = &app.sc;
    let title = tabs(app, theme);
    let block = Block::bordered().title(title).border_style(theme.dim());
    let inner = block.inner(middle);
    frame.render_widget(block, middle);
    let lines = match sc.view {
        View::Status => chain(&sc.chain, theme),
        View::Doctor => doctor(sc, theme),
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    timeline(frame, events, app, theme);
}

fn tabs<'a>(app: &App, theme: &Theme) -> Line<'a> {
    let sc = &app.sc;
    let mut spans = vec![Span::raw(" ")];
    for (view, name) in [(View::Status, "Hub certificates"), (View::Doctor, "Doctor")] {
        if view == sc.view {
            spans.push(Span::styled(format!("[{name}]"), theme.accent()));
        } else {
            spans.push(Span::styled(name, theme.dim()));
        }
        spans.push(Span::raw(" "));
    }
    let note = match &sc.failure {
        Some(last) => format!(
            "| last failure {} ago{} ",
            age_label(app.now, last.at),
            last.hub
                .map_or_else(String::new, |hub| format!(", {} hub", hub_name(hub)))
        ),
        None => "| no failure recorded ".into(),
    };
    spans.push(Span::styled(note, theme.dim()));
    Line::from(spans)
}

fn field<'a>(label: &str, value: String, style: Style, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), theme.dim()),
        Span::styled(value, style),
    ])
}

fn state_word(state: ScConnectionState) -> &'static str {
    match state {
        ScConnectionState::Disconnected => "disconnected",
        ScConnectionState::Connecting => "connecting",
        ScConnectionState::Connected => "connected",
        ScConnectionState::Disconnecting => "disconnecting",
    }
}

fn status_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" Hub connector ", theme.accent()))
        .border_style(theme.dim());
    let sc = &app.sc;
    let Some((link, _)) = &sc.link else {
        let text = match &app.sc.failure {
            Some(_) => "Not connected: the doctor (Tab) shows where the dial failed.",
            None => "Waiting for the hub connector's first report.",
        };
        let lines = Paragraph::new(Line::styled(text, theme.dim())).wrap(Wrap { trim: true });
        frame.render_widget(lines.block(block), area);
        return;
    };
    let style = match link.state {
        ScConnectionState::Connected => theme.accent(),
        ScConnectionState::Connecting => theme.warn(),
        _ => theme.danger(),
    };
    let url = |hub| {
        sc.identity
            .as_ref()
            .and_then(|identity| identity.url(hub))
            .unwrap_or("")
            .to_string()
    };
    let hub = match link.hub {
        Some(hub) => format!("{:<9}{}", hub_name(hub), url(hub)),
        None => "none dialled yet".into(),
    };
    let standby = match sc.identity.as_ref().and_then(|i| i.failover.as_ref()) {
        Some(url) => format!("configured: {url}"),
        None => "none configured".into(),
    };
    let attempt = match sc.attempt {
        Some((hub, attempt, backoff)) => format!(
            "attempt {attempt} to the {} hub after {} ms backoff",
            hub_name(hub),
            backoff.as_millis()
        ),
        None => "none since the last connect".into(),
    };
    let heartbeat = match (sc.heartbeat_age(app.now), link.heartbeat_rtt) {
        (Some(age), Some(rtt)) => format!(
            "last heard {} s ago, round trip {} ms",
            age.as_secs(),
            rtt.as_millis()
        ),
        (Some(age), None) => format!(
            "last heard {} s ago, no heartbeat answered yet",
            age.as_secs()
        ),
        (None, _) => "not connected".into(),
    };
    let lines = vec![
        field("State", state_word(link.state).into(), style, theme),
        field("Hub", hub, theme.accent(), theme),
        field(
            "Hub VMAC",
            link.hub_vmac.map_or("unknown".into(), |v| vmac_text(&v)),
            theme.accent(),
            theme,
        ),
        field("Reconnect", attempt, Style::new(), theme),
        field("Heartbeat", heartbeat, Style::new(), theme),
        field("Failover", standby, theme.dim(), theme),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn identity_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" VMAC and UUID ", theme.accent()))
        .border_style(theme.dim());
    let sc = &app.sc;
    let (vmac, uuid) = match &sc.identity {
        Some(identity) => (vmac_text(&identity.vmac), uuid_text(&identity.uuid)),
        None => ("unknown".into(), "unknown".into()),
    };
    let hub_vmac = sc
        .link
        .as_ref()
        .and_then(|(link, _)| link.hub_vmac)
        .map_or("unknown".into(), |v| vmac_text(&v));
    let mut lines = vec![
        field("Local VMAC", vmac, theme.accent(), theme),
        field("Device UUID", uuid, theme.accent(), theme),
        field("Hub VMAC", hub_vmac, theme.accent(), theme),
    ];
    match sc.duplicates.back() {
        None => lines.push(field("Dup. VMAC", "no NAK seen".into(), theme.dim(), theme)),
        Some(last) => {
            lines.push(field(
                "Dup. VMAC",
                format!(
                    "{} NAK(s), last {} ago",
                    sc.duplicates.len(),
                    age_label(app.now, last.at)
                ),
                theme.danger(),
                theme,
            ));
            let next = if last.reseeded {
                "a new random VMAC was drawn"
            } else {
                "VMAC kept"
            };
            lines.push(Line::styled(format!("{:12}{next}", ""), theme.dim()));
        }
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn certificate<'a>(index: usize, cert: &CertSummary, theme: &Theme) -> Vec<Line<'a>> {
    let role = if index == 0 { "leaf" } else { "issuer" };
    vec![
        Line::from(vec![
            Span::styled(format!("{index} {role:<7}"), theme.dim()),
            Span::styled(format!("subject {}", cert.subject), theme.accent()),
        ]),
        Line::styled(format!("{:10}issued by {}", "", cert.issuer), Style::new()),
        Line::styled(format!("{:10}expires {}", "", cert.not_after), Style::new()),
    ]
}

fn chain<'a>(chain: &[CertSummary], theme: &Theme) -> Vec<Line<'a>> {
    if chain.is_empty() {
        return vec![Line::styled(
            "No hub certificate: not connected to a hub.",
            theme.dim(),
        )];
    }
    let mut lines = vec![Line::styled(
        "As the connected hub presented them, leaf first. Display only: TLS verified the chain.",
        theme.dim(),
    )];
    for (index, cert) in chain.iter().enumerate() {
        lines.extend(certificate(index, cert, theme));
    }
    lines
}

fn mark<'a>(mark: Mark, theme: &Theme) -> Span<'a> {
    match mark {
        Mark::Passed => Span::styled("[ ok ] ", theme.accent()),
        Mark::Failed => Span::styled("[FAIL] ", theme.danger()),
        Mark::NotReached => Span::styled("[ -- ] ", theme.dim()),
        Mark::Unknown => Span::styled("[ ?? ] ", theme.warn()),
    }
}

fn phases<'a>(marks: &[Mark; 6], failed: Option<&Diagnosis>, theme: &Theme) -> Vec<Line<'a>> {
    PHASES
        .iter()
        .zip(marks)
        .map(|(phase, m)| {
            let what = match failed {
                Some(d) if d.failed == Some(*phase) => {
                    Span::styled(d.summary.clone(), theme.danger())
                }
                _ => Span::styled(phase.what(), theme.dim()),
            };
            Line::from(vec![
                mark(*m, theme),
                Span::styled(format!("{:<10}", phase.label()), theme.accent()),
                what,
            ])
        })
        .collect()
}

fn doctor<'a>(sc: &ScState, theme: &Theme) -> Vec<Line<'a>> {
    let connected = sc
        .link
        .as_ref()
        .is_some_and(|(link, _)| link.state == ScConnectionState::Connected);
    let Some(last) = &sc.failure else {
        if connected {
            let mut lines = phases(&CONNECTED, None, theme);
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Connected: every phase passed.",
                theme.accent(),
            ));
            return lines;
        }
        return vec![Line::styled(
            "No failure recorded. A failed connect shows here, phase by phase.",
            theme.dim(),
        )];
    };
    let d = &last.diagnosis;
    let mut lines = phases(&d.marks, Some(d), theme);
    if d.failed.is_none() {
        lines.push(Line::styled(
            format!("Not tied to one phase: {}", d.summary),
            theme.warn(),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Hint    ", theme.warn()),
        Span::raw(d.hint.clone()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Detail  ", theme.dim()),
        Span::styled(last.failure.detail.clone(), theme.dim()),
    ]));
    if let Some(leaf) = last.failure.presented.first() {
        lines.push(Line::from(vec![
            Span::styled("Hub sent ", theme.dim()),
            Span::raw(format!(
                "subject {}, issued by {}, expires {}, signature algorithm {}",
                leaf.subject, leaf.issuer, leaf.not_after, leaf.signature_algorithm
            )),
        ]));
    }
    if connected {
        lines.push(Line::styled(
            "Connected again since: every phase passes now.",
            theme.accent(),
        ));
    }
    lines
}

fn timeline(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let sc = &app.sc;
    let title = if sc.missed > 0 {
        format!(" Events ({} missed) ", sc.missed)
    } else {
        " Events ".into()
    };
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(theme.dim());
    let rows = usize::from(block.inner(area).height);
    let skip = sc.timeline.len().saturating_sub(rows);
    let lines: Vec<Line> = if sc.timeline.is_empty() {
        vec![Line::styled(
            "Connects, losses, reconnect attempts, failover and the primary's return show here.",
            theme.dim(),
        )]
    } else {
        sc.timeline
            .iter()
            .skip(skip)
            .map(|entry| {
                let level = match entry.severity {
                    Severity::Info => Span::raw("INFO "),
                    Severity::Warn => Span::styled("WARN ", theme.warn()),
                    Severity::Error => Span::styled("ERROR", theme.danger()),
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:>5} ago ", age_label(app.now, entry.at)),
                        theme.dim(),
                    ),
                    level,
                    Span::raw(format!(" {}", entry.text)),
                ])
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
