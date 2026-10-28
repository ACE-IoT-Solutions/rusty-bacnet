//! Device strings as the text and Markdown PICS print them (#2137).
//!
//! Vendor_Name, Model_Name, Firmware_Revision and Application_Software_Version
//! come from the Device object, which an application or a custom Device can
//! fill with any text. Each prints inside one line of the document, so a value
//! must not be able to end that line or, in Markdown, its table cell, or start
//! inline markup: otherwise a value could forge rows or sections. Nor may it
//! hide or reorder its own text with a bidirectional or zero-width character.
//! `VendorInfo` keeps the values as read; only the rendering changes them.
//!
//! The rules mirror the CLI's and the TUI's `bacnet_ops::format::clean_name`
//! (#2122), which this crate can't depend on, without its length cap: line
//! breaks become spaces and the [`HIDDEN`] characters are dropped, except the
//! zero width joiner, which only fuses an emoji sequence.

/// The characters dropped from a Device string: the same inclusive ranges as
/// `bacnet_ops::format::clean::HIDDEN`. Every format character (Unicode
/// general category Cf: bidirectional embeddings, overrides, isolates and
/// marks, the zero-width characters, the tag block and the rest), the
/// variation selectors, the Hangul fillers and a few others that show as
/// nothing.
const HIDDEN: &[(char, char)] = &[
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
    // The combining grapheme joiner, the Khmer inherent vowels, the blank
    // Braille pattern and the unassigned code points before the
    // interlinear annotations.
    ('\u{034F}', '\u{034F}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{2800}', '\u{2800}'),
    ('\u{FFF0}', '\u{FFF8}'),
];

/// Whether the PICS drops `c`: a [`HIDDEN`] character other than the zero
/// width joiner (U+200D).
pub(super) fn is_dropped(c: char) -> bool {
    c != '\u{200D}' && HIDDEN.iter().any(|&(low, high)| (low..=high).contains(&c))
}

/// Whether `c` breaks a line: a control character (C0, DEL or C1) or a line
/// or paragraph separator.
fn breaks(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// The value's characters as they print on one line: line breaks as spaces,
/// [`is_dropped`] characters gone.
fn one_line(value: &str) -> impl Iterator<Item = char> + '_ {
    value
        .chars()
        .filter(|c| !is_dropped(*c))
        .map(|c| if breaks(c) { ' ' } else { c })
}

/// The value as one line of the text PICS, as [`one_line`] gives it: it
/// can't start a line of its own or hide or reorder its text.
pub(super) fn text_line(value: &str) -> String {
    one_line(value).collect()
}

/// The value as one Markdown table cell: one line, as [`text_line`] gives it,
/// with a backslash before each character that could end the cell (`|`) or
/// open inline markup: emphasis, code, links, raw HTML, strikethrough or
/// entity references. The backslash itself is escaped too.
pub(super) fn markdown_cell(value: &str) -> String {
    let mut cell = String::with_capacity(value.len());
    for c in one_line(value) {
        if matches!(
            c,
            '\\' | '|' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '&'
        ) {
            cell.push('\\');
        }
        cell.push(c);
    }
    cell
}
