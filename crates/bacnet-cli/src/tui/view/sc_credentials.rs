//! The SC screen's credentials view (#968): this node's operational
//! certificate, chain and CA bundle as the pre-dial lint read them, with the
//! key match, the direct-issuer check and every finding. Each level is a word
//! as well as a colour.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::Theme;
use crate::sc_check::HUB_NAME_NOTE;
use crate::tui::sc::credentials::{Cert, Credentials, Level};

fn style(level: Level, theme: &Theme) -> Style {
    match level {
        Level::Ok => theme.accent(),
        Level::Notice => theme.dim(),
        Level::Warning => theme.warn(),
        Level::Alarm | Level::Failed => theme.danger(),
    }
}

fn mark<'a>(level: Level, theme: &Theme) -> Span<'a> {
    Span::styled(format!("{} ", level.word()), style(level, theme))
}

fn cert<'a>(cert: &Cert, theme: &Theme) -> [Line<'a>; 2] {
    [
        Line::from(vec![
            mark(cert.level, theme),
            Span::styled(format!("{:<14}", cert.label), theme.dim()),
            Span::styled(cert.subject.clone(), theme.accent()),
            Span::styled(
                format!("  {} (until {})", cert.window, cert.not_after),
                style(cert.level, theme),
            ),
        ]),
        Line::styled(
            format!(
                "{:21}issued by {}  key {}  SAN {}  EKU {}",
                "", cert.issuer, cert.key_algorithm, cert.san, cert.eku
            ),
            theme.dim(),
        ),
    ]
}

/// The view's lines; `None` before any SC dial.
pub(super) fn lines<'a>(
    credentials: Option<&Result<Credentials, String>>,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let credentials = match credentials {
        None if cfg!(feature = "sc-tls") => {
            return vec![Line::styled(
                "No dial yet: the credentials are checked just before each one.",
                theme.dim(),
            )]
        }
        None => {
            return vec![Line::styled(
                "Credential checks need a build with the sc-tls feature.",
                theme.dim(),
            )]
        }
        Some(Err(error)) => {
            return vec![
                Line::from(vec![
                    mark(Level::Failed, theme),
                    Span::styled(error.clone(), theme.danger()),
                ]),
                Line::styled("The dial was refused before connecting.", theme.danger()),
            ]
        }
        Some(Ok(credentials)) => credentials,
    };
    let mut lines = vec![Line::styled(
        format!(
            "Checked at {} before the dial. Leaf first, then the CA bundle.",
            credentials.inspected_at
        ),
        theme.dim(),
    )];
    for c in &credentials.certificates {
        lines.extend(cert(c, theme));
    }
    for (label, (level, text)) in [("Key", &credentials.key), ("Issuer", &credentials.issuer)] {
        lines.push(Line::from(vec![
            mark(*level, theme),
            Span::styled(format!("{label:<14}"), theme.dim()),
            Span::styled(text.clone(), style(*level, theme)),
        ]));
    }
    for (level, text) in &credentials.findings {
        lines.push(Line::from(vec![
            mark(*level, theme),
            Span::styled(text.clone(), style(*level, theme)),
        ]));
    }
    if credentials
        .findings
        .iter()
        .any(|(level, _)| *level == Level::Failed)
    {
        lines.push(Line::styled(
            "The dial was refused before connecting: fix the failures above.",
            theme.danger(),
        ));
    }
    lines.push(Line::styled(HUB_NAME_NOTE, theme.dim()));
    lines
}
