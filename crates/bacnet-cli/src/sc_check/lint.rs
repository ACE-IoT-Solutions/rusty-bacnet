//! The pre-dial lint (#968): `bacnet sc-check`'s inspection and severities,
//! applied to this node's own SC credentials before a hub dial.
//!
//! - Hard failures are sc-check's errors: an identity leaf that is expired,
//!   not yet valid or has an invalid interval, an unreadable certificate, and
//!   a key that does not match the leaf or cannot be used. Each one blocks the
//!   dial, in the TUI and in one-shot commands alike, before any socket.
//! - Warnings (expiry within 30 days, a leaf no bundle CA issued directly, a
//!   key file others can read, interpretation limits) can be overridden: the
//!   TUI asks for an explicit confirmation, one-shots log them and go on.
//! - Notices only record what was not assessed.

use super::{projection, Diagnostic};
use bacnet_transport::sc_tls::inspection::{inspect_sc_pem_files, ScPemInspection};
use std::path::Path;

/// How much a finding weighs before a dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    /// Blocks the dial.
    Hard,
    /// Allowed once confirmed (TUI) or logged (one-shots).
    Warning,
    /// Recorded only.
    Notice,
}

/// One finding, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    /// Its weight.
    pub(crate) weight: Weight,
    /// sc-check's diagnostic code, such as `key.mismatch`.
    pub(crate) code: String,
    /// What it applies to and what to do.
    pub(crate) text: String,
}

/// The inspection and its findings.
#[derive(Debug, Clone)]
pub(crate) struct Lint {
    /// The transport's offline report, for the TUI's credentials view.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub(crate) report: ScPemInspection,
    /// sc-check's diagnostics, in its order.
    pub(crate) findings: Vec<Finding>,
}

fn subject(diagnostic: &Diagnostic) -> String {
    let index = diagnostic
        .index
        .map_or(String::new(), |i| format!(" [{i}]"));
    match diagnostic.role {
        Some("identity") => format!("Operational certificate{index}"),
        Some("peer_trust") => format!("CA bundle{index}"),
        Some("private_key") => "Key file".into(),
        _ => "Credentials".into(),
    }
}

impl Lint {
    /// Inspect the files at `at_unix`, or at the clock's time when `None`.
    /// An input that can't be read or decoded is an `Err`, which callers
    /// treat as a hard failure.
    pub(crate) fn run(
        cert: &Path,
        ca: Option<&Path>,
        key: Option<&Path>,
        at_unix: Option<i64>,
    ) -> Result<Self, String> {
        let report = inspect_sc_pem_files(cert, ca, key, at_unix).map_err(|error| {
            let error = projection::input_error(error);
            let mut text = format!("SC credential check could not complete: {}", error.category);
            if let Some(role) = error.role {
                text.push_str(&format!(" {role}"));
            }
            if let Some(kind) = error.kind {
                text.push_str(&format!(" ({kind})"));
            }
            format!("{text}: {}", error.message)
        })?;
        let findings = projection::diagnostics(&report)
            .into_iter()
            .map(|diagnostic| Finding {
                weight: match diagnostic.severity {
                    "error" => Weight::Hard,
                    "warning" => Weight::Warning,
                    _ => Weight::Notice,
                },
                text: format!("{}: {}", subject(&diagnostic), diagnostic.message),
                code: diagnostic.code,
            })
            .collect();
        Ok(Self { report, findings })
    }

    /// The findings of `weight`.
    pub(crate) fn of(&self, weight: Weight) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.weight == weight)
    }

    /// Why the dial is refused, when a hard failure blocks it.
    pub(crate) fn refusal(&self) -> Option<String> {
        let hard: Vec<&str> = self.of(Weight::Hard).map(|f| f.text.as_str()).collect();
        (!hard.is_empty()).then(|| {
            format!(
                "SC credential check refused the dial before connecting: {}",
                hard.join(" ")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z

    /// A self-signed certificate valid from NOW for `days`, and its key.
    fn files(dir: &Path, days: u64) -> (std::path::PathBuf, std::path::PathBuf) {
        let pair = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["node".into()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = params.not_before + std::time::Duration::from_secs(days * 86_400);
        let cert = dir.join("node.pem");
        let key = dir.join("node.key");
        std::fs::write(&cert, params.self_signed(&pair).unwrap().pem()).unwrap();
        std::fs::write(&key, pair.serialize_pem()).unwrap();
        (cert, key)
    }

    fn codes(lint: &Lint, weight: Weight) -> Vec<&str> {
        lint.of(weight).map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn thresholds_weigh_as_alarm_warning_and_ok_and_dates_block() {
        let dir = tempfile::tempdir().unwrap();
        for (days, expected) in [
            (5, Some("expiry.alarm")),
            (25, Some("expiry.warning")),
            (400, None),
        ] {
            let (cert, key) = files(dir.path(), days);
            let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(NOW)).unwrap();
            let warnings = codes(&lint, Weight::Warning);
            let expiry: Vec<_> = warnings
                .iter()
                .filter(|c| c.starts_with("expiry."))
                .collect();
            assert_eq!(expiry.first().map(|c| **c), expected, "{days} days");
            assert!(lint.refusal().is_none(), "{days} days: {:?}", lint.findings);
        }
        let (cert, key) = files(dir.path(), 400);
        for (at, code) in [
            (NOW - 1, "validity.not_yet_valid"),
            (NOW + 401 * 86_400, "validity.expired"),
        ] {
            let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(at)).unwrap();
            assert_eq!(codes(&lint, Weight::Hard), [code]);
            assert!(lint
                .refusal()
                .unwrap()
                .contains("Operational certificate [0]"));
        }
    }

    #[test]
    fn a_mismatched_key_is_hard_and_unreadable_input_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, _) = files(dir.path(), 400);
        let other = dir.path().join("other.key");
        std::fs::write(&other, rcgen::KeyPair::generate().unwrap().serialize_pem()).unwrap();
        let lint = Lint::run(&cert, Some(&cert), Some(&other), Some(NOW)).unwrap();
        assert_eq!(codes(&lint, Weight::Hard), ["key.mismatch"]);
        assert!(lint.refusal().unwrap().contains("Key file:"));
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let error = Lint::run(&empty, None, None, Some(NOW)).unwrap_err();
        assert!(error.contains("empty_certificates"), "{error}");
    }
}
