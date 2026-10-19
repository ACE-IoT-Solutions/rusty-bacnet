//! The device browser: device card, object list, properties and priority
//! array. From the full width the panes sit side by side; below it they
//! stack, and only the focused one is drawn under a strip naming them all.
//! From the wide width the object list grows and the priority array moves
//! beside the properties.
//!
//! The properties pane sizes its name column from the longest name, keeps
//! a gap before each value, and wraps long values under the value column.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{Theme, FULL_WIDTH, WIDE_WIDTH};
use crate::tui::app::browser::{Browser, ListState, Pane, ViewState};
use crate::tui::app::objects::{kebab, object_label, ListRow};
use crate::tui::point::{priority_label, ObjectView, PriorityView, ReadVia, PRIORITIES};

/// Width of the left column (card over objects) in the full layout.
const LEFT_WIDTH: u16 = 46;
/// Height of the priority pane: every slot, Relinquish_Default and
/// Present_Value, and the border.
const PRIORITY_HEIGHT: u16 = PRIORITIES as u16 + 4;
/// Narrowest priority pane beside the properties.
const PRIORITY_WIDTH: u16 = 38;
/// Columns a priority row uses before its value: marker, slot and label.
const PRIORITY_LABEL: usize = 27;
/// Columns between a property name and its value.
const NAME_GAP: usize = 2;
/// The value column always keeps at least this many columns, and at least
/// a third of the pane.
const MIN_VALUE_WIDTH: usize = 12;
/// Width of a card label.
const CARD_LABEL: usize = 18;

/// Draw the browser in `area`.
pub(super) fn draw(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    if area.width >= FULL_WIDTH {
        let wide = area.width >= WIDE_WIDTH;
        let left_width = if wide {
            wide_left_width(area.width, browser)
        } else {
            LEFT_WIDTH
        };
        let [left, right] =
            Layout::horizontal([Constraint::Length(left_width), Constraint::Min(40)]).areas(area);
        let card_height = browser
            .card
            .as_ref()
            .map_or(3, |card| card.rows.len() as u16 + 2);
        let [card, objects] =
            Layout::vertical([Constraint::Length(card_height), Constraint::Min(5)]).areas(left);
        card_pane(frame, card, browser, theme);
        objects_pane(frame, objects, browser, theme);
        object_panes(frame, right, browser, theme, wide);
        return;
    }
    let [strip, pane] = Layout::vertical([Constraint::Length(1), Constraint::Min(5)]).areas(area);
    tab_strip(frame, strip, browser, theme);
    match browser.focus {
        Pane::Card => card_pane(frame, pane, browser, theme),
        Pane::Objects => objects_pane(frame, pane, browser, theme),
        Pane::Properties => object_panes(frame, pane, browser, theme, true),
    }
}

/// The left column in the wide layout: fitted to the card's values, from
/// three tenths of the width up to a third.
fn wide_left_width(width: u16, browser: &Browser) -> u16 {
    let card = browser.card.as_ref().map_or(0, |card| {
        card.rows
            .iter()
            .map(|(_, value)| 1 + CARD_LABEL + text_width(value) + 3)
            .max()
            .unwrap_or(0)
    });
    let card = u16::try_from(card).unwrap_or(u16::MAX);
    card.clamp(width / 10 * 3, width / 3).max(LEFT_WIDTH)
}

fn tab_strip(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    let mut spans = vec![Span::styled(
        format!(" Device {} ", browser.device),
        theme.accent(),
    )];
    for (pane, name) in [
        (Pane::Card, "Card"),
        (Pane::Objects, "Objects"),
        (Pane::Properties, "Properties"),
    ] {
        if pane == browser.focus {
            spans.push(Span::styled(format!(" [{name}] "), theme.selected()));
        } else {
            spans.push(Span::styled(format!("  {name}  "), theme.dim()));
        }
    }
    spans.push(Span::styled("  Tab switches", theme.dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn pane_block<'a>(title: String, focused: bool, theme: &Theme) -> Block<'a> {
    let border = if focused { theme.accent() } else { theme.dim() };
    Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(border)
}

fn card_pane(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    let via = match browser.card.as_ref().and_then(|card| card.via) {
        Some(ReadVia::Rp) => " (via RP)",
        _ => "",
    };
    let title = format!(" Device {} at {}{via} ", browser.device, browser.address);
    let block = pane_block(title, browser.focus == Pane::Card, theme);
    let lines: Vec<Line> = match &browser.card {
        None => vec![Line::styled(" Reading the device card...", theme.dim())],
        Some(card) => card
            .rows
            .iter()
            .map(|(label, value)| {
                Line::from(vec![
                    Span::styled(format!(" {label:<CARD_LABEL$}"), theme.dim()),
                    Span::raw(value.clone()),
                ])
            })
            .collect(),
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn list_status(browser: &Browser) -> String {
    match &browser.list {
        ListState::Card => " reading the device ".into(),
        ListState::Reading { done, total } => format!(" reading Object_List {done}/{total} "),
        ListState::Ready { indexed: true } => " Object_List read element by element ".into(),
        ListState::Ready { indexed: false } => String::new(),
        ListState::Partial(error) => format!(" Object_List stopped at {error} "),
        ListState::Failed(error) => format!(" failed: {error} "),
    }
}

fn objects_pane(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    let objects = &browser.objects;
    let count = if objects.shown() == objects.len() {
        objects.len().to_string()
    } else {
        format!("{} of {}", objects.shown(), objects.len())
    };
    let mut block = pane_block(
        format!(" Objects ({count}) "),
        browser.focus == Pane::Objects,
        theme,
    );
    let status = list_status(browser);
    if !status.is_empty() {
        block = block.title_bottom(Line::from(status));
    }
    if !objects.filter().is_empty() {
        block = block.title(
            Line::styled(format!(" filter: \"{}\" ", objects.filter()), theme.warn())
                .right_aligned(),
        );
    }
    let height = usize::from(block.inner(area).height).max(1);
    objects.page.set(height);
    let selected = objects.selected_index();
    let mut offset = objects.offset.get();
    if let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    offset = offset.min(objects.rows().len().saturating_sub(height));
    objects.offset.set(offset);
    let lines: Vec<Line> = objects
        .rows()
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, row)| match row {
            ListRow::Group { object_type, count } => {
                Line::styled(format!(" {} ({count})", kebab(object_type)), theme.accent())
            }
            ListRow::Object(object) if selected == Some(index) => {
                Line::styled(format!(">  {}", object_label(*object)), theme.selected())
            }
            ListRow::Object(object) => Line::raw(format!("   {}", object_label(*object))),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The properties pane, with the priority pane beside it (`beside`) or
/// below it for a commandable object.
fn object_panes(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme, beside: bool) {
    let priority = match &browser.view {
        ViewState::Ready(view) => view.priority.as_ref(),
        _ => None,
    };
    let Some(priority) = priority else {
        properties_pane(frame, area, browser, theme);
        return;
    };
    let [properties, slots] = if beside {
        let width = priority_width(priority)
            .min(area.width / 2)
            .max(PRIORITY_WIDTH);
        Layout::horizontal([Constraint::Min(30), Constraint::Length(width)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(5), Constraint::Length(PRIORITY_HEIGHT)]).areas(area)
    };
    properties_pane(frame, properties, browser, theme);
    priority_pane(frame, slots, priority, theme);
}

fn properties_pane(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    let focused = browser.focus == Pane::Properties;
    // Inside the border.
    let inner = usize::from(area.width.saturating_sub(2));
    let (title, lines) = match &browser.view {
        ViewState::Empty => (
            " Properties ".to_string(),
            vec![Line::styled(
                " Select an object and press Enter.",
                theme.dim(),
            )],
        ),
        ViewState::Reading(object) => (
            format!(" {} ", object_label(*object)),
            vec![Line::styled(" Reading...", theme.dim())],
        ),
        ViewState::Failed(object, error) => (
            format!(" {} ", object_label(*object)),
            // A space each side, as for a property row.
            wrap(&format!("Read failed: {error}"), inner.saturating_sub(2))
                .into_iter()
                .map(|line| Line::styled(format!(" {line}"), theme.danger()))
                .collect(),
        ),
        ViewState::Ready(view) => (title(view), property_lines(view, inner, theme)),
    };
    let block = pane_block(title, focused, theme);
    let height = usize::from(block.inner(area).height).max(1);
    let scroll = browser.scroll.get().min(lines.len().saturating_sub(height));
    browser.scroll.set(scroll);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(height).collect();
    frame.render_widget(Paragraph::new(shown).block(block), area);
}

fn title(view: &ObjectView) -> String {
    let via = match view.via {
        ReadVia::Rpm => "",
        ReadVia::Rp => " (via RP)",
    };
    format!(" {}{via} ", object_label(view.object))
}

/// Display width of `text`.
fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Width of the name column for names `widths` wide in a pane `inner`
/// columns wide: the longest name, capped so the value column keeps its
/// room. A longer name puts its value on the next line.
fn name_width(widths: impl Iterator<Item = usize>, inner: usize) -> usize {
    let value_room = (inner / 3).max(MIN_VALUE_WIDTH);
    // A space each side of the row.
    let cap = inner.saturating_sub(2 + NAME_GAP + value_room);
    widths.max().unwrap_or(0).min(cap)
}

/// `text` broken into lines at most `width` columns wide, at spaces where
/// it can be and inside a word longer than a line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    // Trailing spaces would only add a blank line.
    for word in text.trim_end().split(' ') {
        let word_width = text_width(word);
        if used > 0 && used + 1 + word_width <= width {
            line.push(' ');
            line.push_str(word);
            used += 1 + word_width;
            continue;
        }
        if used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        for ch in word.chars() {
            let ch_width = text_width(ch.encode_utf8(&mut [0; 4]));
            if used > 0 && used + ch_width > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(ch);
            used += ch_width;
        }
    }
    lines.push(line);
    lines
}

/// The status line and one row per property for a pane `inner` columns
/// wide, long values wrapped under the value column.
fn property_lines(view: &ObjectView, inner: usize, theme: &Theme) -> Vec<Line<'static>> {
    let badges = view.health.badges();
    let mut status = vec![Span::styled(" Status: ", theme.dim())];
    if badges.is_empty() {
        status.push(Span::raw("normal"));
    }
    for badge in badges {
        let style = match badge {
            "[ALM]" | "[FLT]" => theme.danger(),
            _ => theme.warn(),
        };
        status.push(Span::styled(badge, style));
        status.push(Span::raw(" "));
    }
    let mut lines = vec![Line::from(status)];
    let names: Vec<String> = view
        .properties
        .iter()
        .map(|row| kebab(row.property))
        .collect();
    let column = name_width(names.iter().map(|name| text_width(name)), inner);
    // A space each side of the row.
    let value_width = inner.saturating_sub(2 + column + NAME_GAP);
    let indent = " ".repeat(1 + column + NAME_GAP);
    for (row, name) in view.properties.iter().zip(names) {
        let (text, style) = match &row.value {
            Ok(value) => (value.clone(), Style::new()),
            Err(error) => (format!("({error})"), theme.dim()),
        };
        let mut chunks = wrap(&text, value_width).into_iter();
        let pad = column.checked_sub(text_width(&name));
        let mut first = vec![Span::styled(format!(" {name}"), theme.dim())];
        if let Some(pad) = pad {
            first.push(Span::raw(" ".repeat(pad + NAME_GAP)));
            first.push(Span::styled(chunks.next().unwrap_or_default(), style));
        }
        lines.push(Line::from(first));
        for chunk in chunks {
            lines.push(Line::from(vec![
                Span::raw(indent.clone()),
                Span::styled(chunk, style),
            ]));
        }
    }
    lines
}

/// Width of the priority pane beside the properties, fitted to its values.
fn priority_width(priority: &PriorityView) -> u16 {
    let longest = priority
        .slots
        .iter()
        .chain([&priority.relinquish_default, &priority.present_value])
        .map(|value| value.as_deref().map_or(1, text_width))
        .max()
        .unwrap_or(1);
    // A space after the value, and the border.
    u16::try_from(PRIORITY_LABEL + longest + 3).unwrap_or(u16::MAX)
}

fn priority_pane(frame: &mut Frame, area: Rect, priority: &PriorityView, theme: &Theme) {
    let winner = priority.winner();
    let title = match winner {
        Some(slot) => format!(" Priority array: {slot} in control "),
        None => " Priority array: relinquished ".to_string(),
    };
    let block = pane_block(title, false, theme);
    let mut lines: Vec<Line> = priority
        .slots
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let slot = index + 1;
            let marker = if winner == Some(slot) { ">" } else { " " };
            let text = format!(
                "{marker}{slot:>2} {:<22} {}",
                priority_label(slot),
                value.as_deref().unwrap_or("-")
            );
            if winner == Some(slot) {
                Line::styled(text, theme.selected())
            } else if value.is_some() {
                Line::raw(text)
            } else {
                Line::styled(text, theme.dim())
            }
        })
        .collect();
    let default_style = if winner.is_none() {
        theme.selected()
    } else {
        Style::new()
    };
    lines.push(Line::styled(
        format!(
            " Relinquish_Default  {}",
            priority.relinquish_default.as_deref().unwrap_or("-")
        ),
        default_style,
    ));
    lines.push(Line::raw(format!(
        " Present_Value       {}",
        priority.present_value.as_deref().unwrap_or("-")
    )));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_column_fits_the_longest_name_up_to_its_cap() {
        // Room for every name.
        assert_eq!(name_width([17, 30, 12].into_iter(), 72), 30);
        // A wide pane never pads past the longest name.
        assert_eq!(name_width([5].into_iter(), 200), 5);
        // Narrow: the value keeps a third of the pane (at least 12).
        assert_eq!(name_width([60].into_iter(), 72), 72 - 2 - NAME_GAP - 24);
        assert_eq!(name_width([30].into_iter(), 30), 30 - 2 - NAME_GAP - 12);
        assert_eq!(name_width(std::iter::empty(), 72), 0);
    }

    #[test]
    fn wrap_breaks_at_spaces_and_inside_long_words() {
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("short", 10), ["short"]);
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("72.5 °F", 4), ["72.5", "°F"]);
        assert_eq!(wrap("ab    ", 2), ["ab"]);
        for line in wrap("[a, bb, ccc, dddd, eeeee, ffffff]", 9) {
            assert!(text_width(&line) <= 9, "{line:?}");
        }
    }
}
