//! Device strings as the text and Markdown PICS print them (#2137).
//!
//! Vendor_Name, Model_Name, Firmware_Revision and Application_Software_Version
//! come from the Device object, which an application or a custom Device can
//! fill with any text. Each prints inside one line of the document, so a value
//! must not be able to end that line or, in Markdown, its table cell, or start
//! inline markup: otherwise a value could forge rows or sections. `VendorInfo`
//! keeps the values as read; only the rendering changes them.

/// A control character, line breaks included, becomes a space.
fn one_line(c: char) -> char {
    if c.is_control() { ' ' } else { c }
}

/// The value as one line of the text PICS: control characters, line breaks
/// among them, become spaces, so the value can't start a line of its own.
pub(super) fn text_line(value: &str) -> String {
    value.chars().map(one_line).collect()
}

/// The value as one Markdown table cell: one line, as [`text_line`] gives it,
/// with a backslash before each character that could end the cell (`|`) or
/// open inline markup: emphasis, code, links, raw HTML, strikethrough or
/// entity references. The backslash itself is escaped too.
pub(super) fn markdown_cell(value: &str) -> String {
    let mut cell = String::with_capacity(value.len());
    for c in value.chars().map(one_line) {
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
