//! Device-supplied text, labelled as data (design §7.5).
//!
//! Anyone who can configure a device, or spoof one, sets its names and
//! descriptions; B/IP has no authentication. So every string from the wire
//! comes back as an envelope, never spliced into prose:
//! `{"untrusted": "...", "source": "device 1201 analog-output:3 description",
//! "truncated": false}`. The text inside is cleaned first: control
//! characters (C0 and C1) become spaces, bidirectional overrides and
//! invisible formatting characters are dropped, URLs are defanged
//! (`hxxp://`), and the result is capped.

use serde_json::{Value, json};

/// A device-supplied string as an envelope: cleaned, capped at `cap`
/// characters, and labelled with where it came from.
pub(crate) fn envelope(text: &str, source: impl Into<String>, cap: usize) -> Value {
    let (text, truncated) = clean(text, cap);
    json!({ "untrusted": text, "source": source.into(), "truncated": truncated })
}

/// `text` cleaned and capped; true when it was cut.
pub(crate) fn clean(text: &str, cap: usize) -> (String, bool) {
    let mut cleaned: String = text
        .chars()
        .filter(|ch| !hidden(*ch))
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    cleaned = defang(&cleaned);
    match cleaned.char_indices().nth(cap) {
        Some((cut, _)) => {
            cleaned.truncate(cut);
            (cleaned, true)
        }
        None => (cleaned, false),
    }
}

/// Characters that change how text around them reads without showing:
/// the bidirectional embeddings, overrides and isolates (U+202A to U+202E,
/// U+2066 to U+2069), the directional marks, and the zero-width ones.
fn hidden(ch: char) -> bool {
    matches!(
        ch,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200B}'..='\u{200F}'
            | '\u{061C}'
            | '\u{2060}'
            | '\u{FEFF}'
    )
}

/// URLs made inert: `http://` and `https://` as `hxxp://` and `hxxps://`,
/// and any other `scheme://` as `scheme[:]//`, so no host renders them as
/// links.
fn defang(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at);
        let scheme_start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')))
            .map_or(0, |(i, ch)| i + ch.len_utf8());
        let (lead, scheme) = before.split_at(scheme_start);
        out.push_str(lead);
        match scheme.to_ascii_lowercase().as_str() {
            "http" | "https" => {
                // Keep the scheme's own case after the h: HTTP becomes HXXP.
                let x = if scheme.as_bytes()[1].is_ascii_uppercase() {
                    "XX"
                } else {
                    "xx"
                };
                out.push_str(&scheme[..1]);
                out.push_str(x);
                out.push_str(&scheme[3..]);
                out.push_str("://");
            }
            _ => {
                out.push_str(scheme);
                out.push_str("[:]//");
            }
        }
        rest = &after[3..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_bidi_and_urls_are_neutralised() {
        let text = "Supply\u{0007} Fan\u{009B}\u{202E}gnp.exe\u{202C} see https://evil.example/x \
                    and ftp://h/f or HTTP://A";
        let (cleaned, truncated) = clean(text, 256);
        assert!(!truncated);
        assert_eq!(
            cleaned,
            "Supply  Fan gnp.exe see hxxps://evil.example/x and ftp[:]//h/f or HXXP://A"
        );
        assert!(!cleaned.chars().any(|ch| ch.is_control() || hidden(ch)));
    }

    #[test]
    fn long_text_is_capped_by_characters() {
        let text = "é".repeat(300);
        let (cleaned, truncated) = clean(&text, 256);
        assert!(truncated);
        assert_eq!(cleaned.chars().count(), 256);
        let value = envelope("ok", "device 1 name", 256);
        assert_eq!(
            value,
            json!({ "untrusted": "ok", "source": "device 1 name", "truncated": false })
        );
    }

    #[test]
    fn a_bare_separator_is_left_alone() {
        assert_eq!(defang("://x"), "[:]//x");
        assert_eq!(defang("é://x éhttp://y"), "é[:]//x éhxxp://y");
        assert_eq!(defang("no url here"), "no url here");
    }
}
