//! Device text made safe to show: names, state texts and other strings a
//! device supplies, which anyone who can configure (or spoof) it sets.
//!
//! A control character can move a terminal's cursor or break a table row,
//! and a format character (a bidirectional override or isolate, a
//! zero-width one) changes how the text around it reads without showing
//! itself: `High` followed by a right-to-left override reverses the rest
//! of the row. [`clean_name`] turns the first into spaces and drops the
//! second, for the CLI and the TUI; the MCP server's untrusted envelopes
//! drop the same [`HIDDEN`] set.

/// The characters dropped from device text: every format character
/// (general category Cf, Unicode 16: the bidirectional embeddings,
/// overrides, isolates and marks, the zero-width ones, the tag characters
/// and the rest), the variation selectors, the Hangul fillers, and a few
/// others that show as nothing. Inclusive ranges.
pub const HIDDEN: &[(char, char)] = &[
    // Format characters (Cf).
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    // The tag block, Cf and unassigned alike.
    ('\u{E0000}', '\u{E007F}'),
    // Variation selectors, Mongolian ones included.
    ('\u{180B}', '\u{180D}'),
    ('\u{180F}', '\u{180F}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{E0100}', '\u{E01EF}'),
    // Hangul fillers.
    ('\u{115F}', '\u{1160}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FFA0}', '\u{FFA0}'),
    // Others that show as nothing: the combining grapheme joiner, the
    // Khmer inherent vowels, the blank Braille pattern, and the
    // unassigned code points before the interlinear annotations.
    ('\u{034F}', '\u{034F}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{2800}', '\u{2800}'),
    ('\u{FFF0}', '\u{FFF8}'),
];

/// Whether `ch` is in [`HIDDEN`].
pub fn is_hidden(ch: char) -> bool {
    HIDDEN.iter().any(|&(low, high)| (low..=high).contains(&ch))
}

/// Whether a screen drops `ch`: a [`HIDDEN`] character other than the zero
/// width joiner (U+200D). The joiner only fuses an emoji sequence (a
/// family, a flag) into one picture, and can't reorder or hide the text
/// around it, so the CLI and the TUI keep it; the MCP envelopes, read by a
/// model rather than a person, drop it with the rest.
pub fn is_hidden_on_screen(ch: char) -> bool {
    ch != '\u{200D}' && is_hidden(ch)
}

/// Whether `ch` breaks a line or moves the cursor: a control (C0, DEL and
/// C1) or a line or paragraph separator.
fn breaks(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')
}

/// Longest name kept, in characters; a device's longer name is cut.
pub const MAX_NAME_CHARS: usize = 128;

/// A device's name (or other short text) for display: each control
/// character and line or paragraph separator as a space, so it can't move
/// the cursor or break a row, each character [`is_hidden_on_screen`]
/// dropped, so it can't reorder or hide what follows, and at most
/// [`MAX_NAME_CHARS`] characters.
pub fn clean_name(text: &str) -> String {
    text.chars()
        .filter(|ch| !is_hidden_on_screen(*ch))
        .take(MAX_NAME_CHARS)
        .map(|ch| if breaks(ch) { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Controls become spaces; bidirectional overrides and isolates
    /// (U+202E, U+2066 to U+2069) and the zero-width ones are dropped, so
    /// they can't reverse or hide the rest of a row (#2122).
    #[test]
    fn controls_become_spaces_and_format_characters_go() {
        assert_eq!(clean_name("Zone\nTemp\u{1b}"), "Zone Temp ");
        assert_eq!(clean_name("a\u{2028}b\u{2029}c"), "a b c");
        assert_eq!(clean_name("High\u{202E} 1"), "High 1");
        assert_eq!(
            clean_name("\u{2066}a\u{2067}b\u{2068}c\u{2069}\u{200B}d\u{FEFF}"),
            "abcd"
        );
        // The zero width joiner keeps an emoji family one picture.
        let family = "\u{1F468}\u{200D}\u{1F469}";
        assert_eq!(clean_name(family), family);
        for &(low, high) in HIDDEN {
            for ch in [low, high].into_iter().filter(|ch| *ch != '\u{200D}') {
                assert_eq!(
                    clean_name(&format!("a{ch}b")),
                    "ab",
                    "U+{:04X}",
                    u32::from(ch)
                );
            }
        }
        // Dropped characters don't count toward the cap.
        let long = format!("{}{}", "\u{202E}".repeat(10), "é".repeat(500));
        assert_eq!(clean_name(&long), "é".repeat(MAX_NAME_CHARS));
    }
}
