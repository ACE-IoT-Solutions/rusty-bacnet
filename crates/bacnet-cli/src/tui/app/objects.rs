//! The browser's object list: a device's Object_List grouped by object
//! type, with a filter and a selection that skips the group headings.

use std::cell::Cell;
use std::collections::BTreeMap;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::devices::Movement;

/// `ANALOG_INPUT` as `analog-input`.
pub(crate) fn kebab(name: impl std::fmt::Display) -> String {
    name.to_string().to_lowercase().replace('_', "-")
}

/// An object as the list shows it, such as `analog-input 1`.
pub(crate) fn object_label(object: ObjectIdentifier) -> String {
    format!(
        "{} {}",
        kebab(object.object_type()),
        object.instance_number()
    )
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListRow {
    /// A type heading with the number of its objects shown.
    Group {
        /// The type.
        object_type: ObjectType,
        /// Objects of it that pass the filter.
        count: usize,
    },
    /// An object.
    Object(ObjectIdentifier),
}

/// The grouped, filtered object list.
#[derive(Default)]
pub(crate) struct ObjectList {
    objects: Vec<ObjectIdentifier>,
    filter: String,
    rows: Vec<ListRow>,
    /// Index into `rows`, always an object row.
    selected: Option<usize>,
    /// First visible row, kept by the view between frames.
    pub(crate) offset: Cell<usize>,
    /// Rows that fit on screen in the last frame.
    pub(crate) page: Cell<usize>,
}

impl ObjectList {
    /// Replace the objects, keeping the selection when it is still there.
    pub(crate) fn set(&mut self, objects: Vec<ObjectIdentifier>) {
        self.objects = objects;
        self.rebuild();
    }

    /// Objects held, filtered or not.
    pub(crate) fn len(&self) -> usize {
        self.objects.len()
    }

    /// Objects that pass the filter.
    pub(crate) fn shown(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, ListRow::Object(_)))
            .count()
    }

    /// The lines to draw.
    pub(crate) fn rows(&self) -> &[ListRow] {
        &self.rows
    }

    /// Index of the selected line.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// The selected object.
    pub(crate) fn selected(&self) -> Option<ObjectIdentifier> {
        match self.rows.get(self.selected?)? {
            ListRow::Object(object) => Some(*object),
            ListRow::Group { .. } => None,
        }
    }

    /// The filter text.
    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    /// Append to the filter.
    pub(crate) fn push_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.rebuild();
    }

    /// Remove the filter's last character.
    pub(crate) fn pop_filter(&mut self) {
        self.filter.pop();
        self.rebuild();
    }

    /// Clear the filter.
    pub(crate) fn clear_filter(&mut self) {
        self.filter.clear();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let keep = self.selected();
        let needle = self.filter.to_lowercase();
        let mut groups: BTreeMap<u32, Vec<ObjectIdentifier>> = BTreeMap::new();
        for &object in &self.objects {
            if needle.is_empty() || object_label(object).contains(&needle) {
                groups
                    .entry(object.object_type().to_raw())
                    .or_default()
                    .push(object);
            }
        }
        self.rows.clear();
        for (raw, mut members) in groups {
            members.sort_by_key(|object| object.instance_number());
            self.rows.push(ListRow::Group {
                object_type: ObjectType::from_raw(raw),
                count: members.len(),
            });
            self.rows.extend(members.into_iter().map(ListRow::Object));
        }
        self.selected = keep
            .and_then(|object| {
                self.rows
                    .iter()
                    .position(|row| *row == ListRow::Object(object))
            })
            .or_else(|| self.object_rows().next());
    }

    fn object_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, ListRow::Object(_)))
            .map(|(index, _)| index)
    }

    /// Move the selection among the objects.
    pub(crate) fn move_selection(&mut self, movement: Movement) {
        let objects: Vec<usize> = self.object_rows().collect();
        let Some(last) = objects.len().checked_sub(1) else {
            return;
        };
        let current = self
            .selected
            .and_then(|row| objects.iter().position(|&index| index == row))
            .unwrap_or(0);
        let page = self.page.get().max(1);
        let next = match movement {
            Movement::Up => current.saturating_sub(1),
            Movement::Down => (current + 1).min(last),
            Movement::PageUp => current.saturating_sub(page),
            Movement::PageDown => (current + page).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
        self.selected = Some(objects[next]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(object_type, instance).unwrap()
    }

    #[test]
    fn objects_are_grouped_by_type_and_the_selection_skips_headings() {
        let mut list = ObjectList::default();
        list.set(vec![
            oid(ObjectType::DEVICE, 9),
            oid(ObjectType::ANALOG_INPUT, 2),
            oid(ObjectType::ANALOG_OUTPUT, 1),
            oid(ObjectType::ANALOG_INPUT, 1),
        ]);
        assert_eq!(list.rows().len(), 7);
        assert_eq!(list.selected(), Some(oid(ObjectType::ANALOG_INPUT, 1)));
        list.move_selection(Movement::Down);
        list.move_selection(Movement::Down);
        assert_eq!(list.selected(), Some(oid(ObjectType::ANALOG_OUTPUT, 1)));
        list.move_selection(Movement::End);
        assert_eq!(list.selected(), Some(oid(ObjectType::DEVICE, 9)));
        // The filter keeps the selection when it still matches.
        list.push_filter('d');
        assert_eq!(list.shown(), 1);
        assert_eq!(list.selected(), Some(oid(ObjectType::DEVICE, 9)));
        list.clear_filter();
        assert_eq!(list.shown(), 4);
    }
}
