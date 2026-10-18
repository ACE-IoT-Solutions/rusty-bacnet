//! The device browser: device card, object list, properties and priority
//! array. From the full width the panes sit side by side; below it they
//! stack, and only the focused one is drawn under a strip naming them all.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{Theme, FULL_WIDTH};
use crate::tui::app::browser::{Browser, ListState, Pane, ViewState};
use crate::tui::app::objects::{kebab, object_label, ListRow};
use crate::tui::point::{priority_label, ObjectView, PriorityView, ReadVia, PRIORITIES};

/// Width of the left column (card over objects) in the full layout.
const LEFT_WIDTH: u16 = 46;
/// Height of the priority pane: every slot, Relinquish_Default and
/// Present_Value, and the border.
const PRIORITY_HEIGHT: u16 = PRIORITIES as u16 + 4;

/// Draw the browser in `area`.
pub(super) fn draw(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    if area.width >= FULL_WIDTH {
        let [left, right] =
            Layout::horizontal([Constraint::Length(LEFT_WIDTH), Constraint::Min(40)]).areas(area);
        let card_height = browser
            .card
            .as_ref()
            .map_or(3, |card| card.rows.len() as u16 + 2);
        let [card, objects] =
            Layout::vertical([Constraint::Length(card_height), Constraint::Min(5)]).areas(left);
        card_pane(frame, card, browser, theme);
        objects_pane(frame, objects, browser, theme);
        object_panes(frame, right, browser, theme, false);
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
                    Span::styled(format!(" {label:<18}"), theme.dim()),
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
        Layout::horizontal([Constraint::Min(30), Constraint::Length(38)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(5), Constraint::Length(PRIORITY_HEIGHT)]).areas(area)
    };
    properties_pane(frame, properties, browser, theme);
    priority_pane(frame, slots, priority, theme);
}

fn properties_pane(frame: &mut Frame, area: Rect, browser: &Browser, theme: &Theme) {
    let focused = browser.focus == Pane::Properties;
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
            vec![Line::styled(
                format!(" Read failed: {error}"),
                theme.danger(),
            )],
        ),
        ViewState::Ready(view) => (title(view), property_lines(view, theme)),
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

fn property_lines(view: &ObjectView, theme: &Theme) -> Vec<Line<'static>> {
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
    for row in &view.properties {
        let name = Span::styled(format!(" {:<28}", kebab(row.property)), theme.dim());
        let value = match &row.value {
            Ok(value) => Span::raw(value.clone()),
            Err(error) => Span::styled(format!("({error})"), theme.dim()),
        };
        lines.push(Line::from(vec![name, value]));
    }
    lines
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
