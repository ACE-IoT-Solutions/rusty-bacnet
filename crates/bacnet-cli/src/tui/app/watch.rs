//! The watch list: points followed live, by COV or by polling, each with a
//! short history for its sparkline.
//!
//! Rows are added from the browser (`a`) or the command bar (`watch ...`)
//! and removed with `x` on the watch screen. Every add and remove goes to
//! the watch worker, which subscribes, polls and cancels; the app keeps
//! only what it shows.

use std::collections::VecDeque;
use std::time::Instant;

use bacnet_types::enums::PropertyIdentifier;

use super::devices::Movement;
use super::objects::object_label;
use super::{App, Link, FLASH_FOR};
use crate::tui::message::Command;
use crate::tui::point::Health;
use crate::tui::watched::{Point, RowId, Source, WatchCommand, WatchEvent, WatchStats};
use crate::tui::worker::watch::MAX_ROWS;

/// Values a row keeps for its sparkline.
pub(crate) const HISTORY: usize = 120;

/// The screen in the body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Screen {
    /// The device table, or the browser when one is open.
    #[default]
    Main,
    /// The watch list.
    Watch,
    /// B/IP infrastructure: foreign-device status, BDT, FDT and the matrix.
    Network,
}

/// One watched point as shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WatchRow {
    /// Its id, which the worker also uses.
    pub(crate) id: RowId,
    /// What it watches.
    pub(crate) point: Point,
    /// Where its values come from.
    pub(crate) source: Source,
    /// The latest value, or why it couldn't be read.
    pub(crate) value: Option<Result<String, String>>,
    /// Units, when the object has them.
    pub(crate) units: Option<String>,
    /// The latest Status_Flags.
    pub(crate) flags: Option<Health>,
    /// When the latest value arrived.
    pub(crate) updated: Option<Instant>,
    /// Recent numeric values, oldest first.
    pub(crate) history: VecDeque<f64>,
}

/// The watch list's state.
#[derive(Debug, Default)]
pub(crate) struct WatchList {
    rows: Vec<WatchRow>,
    selected: usize,
    next_id: RowId,
    /// The worker's latest counters.
    pub(crate) stats: WatchStats,
}

impl WatchList {
    /// The rows, in the order they were added.
    pub(crate) fn rows(&self) -> &[WatchRow] {
        &self.rows
    }

    /// The selected row's index.
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// Add a row for `point`, returning its id.
    fn add(&mut self, point: Point) -> Result<RowId, String> {
        if self.rows.iter().any(|row| row.point == point) {
            return Err("Already watched.".into());
        }
        if self.rows.len() >= MAX_ROWS {
            return Err(format!("The watch list holds at most {MAX_ROWS} rows."));
        }
        self.next_id += 1;
        self.rows.push(WatchRow {
            id: self.next_id,
            point,
            source: Source::Starting,
            value: None,
            units: None,
            flags: None,
            updated: None,
            history: VecDeque::new(),
        });
        Ok(self.next_id)
    }

    /// Remove the selected row, returning its id.
    fn remove_selected(&mut self) -> Option<RowId> {
        if self.selected >= self.rows.len() {
            return None;
        }
        let row = self.rows.remove(self.selected);
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        Some(row.id)
    }

    fn row_mut(&mut self, id: RowId) -> Option<&mut WatchRow> {
        self.rows.iter_mut().find(|row| row.id == id)
    }

    /// Apply a worker report; returns a message for the footer.
    pub(crate) fn apply(&mut self, event: WatchEvent) -> Option<String> {
        match event {
            WatchEvent::Source { row, source } => {
                if let Source::Failed(error) = &source {
                    let text =
                        self.rows.iter().find(|r| r.id == row).map(|r| {
                            format!("Can't watch {}: {error}", object_label(r.point.object))
                        });
                    self.rows.retain(|r| r.id != row);
                    self.selected = self.selected.min(self.rows.len().saturating_sub(1));
                    return text;
                }
                if let Some(row) = self.row_mut(row) {
                    row.source = source;
                }
            }
            WatchEvent::Units { row, units } => {
                if let Some(row) = self.row_mut(row) {
                    row.units = Some(units);
                }
            }
            WatchEvent::Samples(samples) => {
                for (id, sample) in samples {
                    let Some(row) = self.row_mut(id) else {
                        continue;
                    };
                    if let Some(number) = sample.number {
                        if row.history.len() >= HISTORY {
                            row.history.pop_front();
                        }
                        row.history.push_back(number);
                    }
                    row.flags = sample.flags.or(row.flags);
                    row.value = Some(sample.value);
                    row.updated = Some(sample.at);
                }
            }
            WatchEvent::Stats(stats) => self.stats = stats,
            WatchEvent::Notice(text) => return Some(text),
        }
        None
    }

    /// Move the selection.
    pub(crate) fn move_selection(&mut self, movement: Movement) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = match movement {
            Movement::Up => self.selected.saturating_sub(1),
            Movement::Down => (self.selected + 1).min(last),
            Movement::PageUp => self.selected.saturating_sub(10),
            Movement::PageDown => (self.selected + 10).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
    }
}

impl App {
    /// Watch `point`: add its row and ask the worker to start it.
    pub(super) fn watch_point(&mut self, point: Point, out: &mut Vec<Command>) {
        if !self.may_transmit() {
            return;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected.", FLASH_FOR);
            return;
        }
        match self.watch.add(point) {
            Ok(row) => {
                out.push(Command::Watch(WatchCommand::Add { row, point }));
                self.flash(
                    format!(
                        "Watching {} on device {}. Press 3 for the watch list.",
                        object_label(point.object),
                        point.device
                    ),
                    FLASH_FOR,
                );
            }
            Err(error) => self.flash(error, FLASH_FOR),
        }
    }

    /// `a` in the browser: watch the selected object's Present_Value.
    pub(super) fn watch_selected(&mut self, out: &mut Vec<Command>) {
        let Some((device, object)) = self.browser.as_deref().and_then(|b| b.selected_point())
        else {
            self.flash("Open a device and select an object to watch.", FLASH_FOR);
            return;
        };
        let point = Point {
            device,
            object,
            property: PropertyIdentifier::PRESENT_VALUE,
        };
        self.watch_point(point, out);
    }

    /// `x` on the watch screen: stop watching the selected row.
    pub(super) fn unwatch_selected(&mut self, out: &mut Vec<Command>) {
        if let Some(row) = self.watch.remove_selected() {
            out.push(Command::Watch(WatchCommand::Remove { row }));
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;
    use crate::tui::watched::Sample;

    fn point(instance: u32) -> Point {
        Point {
            device: 7,
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
        }
    }

    #[test]
    fn rows_are_unique_and_the_history_is_bounded() {
        let mut list = WatchList::default();
        let id = list.add(point(1)).unwrap();
        assert!(list.add(point(1)).is_err(), "a duplicate");
        let now = Instant::now();
        let samples = (0..HISTORY + 5)
            .map(|n| {
                (
                    id,
                    Sample {
                        value: Ok(n.to_string()),
                        number: Some(n as f64),
                        flags: None,
                        at: now,
                    },
                )
            })
            .collect();
        list.apply(WatchEvent::Samples(samples));
        let row = &list.rows()[0];
        assert_eq!(row.history.len(), HISTORY);
        assert_eq!(row.history.front(), Some(&5.0));
        assert_eq!(row.value.as_ref().unwrap().as_deref(), Ok("124"));
        // A row the worker can't watch goes, with a message.
        let notice = list.apply(WatchEvent::Source {
            row: id,
            source: Source::Failed("no such device".into()),
        });
        assert!(notice.unwrap().contains("no such device"));
        assert!(list.rows().is_empty());
    }
}
