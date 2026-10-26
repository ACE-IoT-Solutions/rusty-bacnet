//! A log's values named by the property it samples (#2091).
//!
//! A Trend Log record carries a bare value: an enumeration is a number and
//! a bit string its bits, with nothing to say which property they came
//! from. The log's Log_DeviceObjectProperty does say: one reference for a
//! Trend Log (Clause 12.25), one per member for a Trend Log Multiple
//! (Clause 12.30), each in the order of the record's values. Read once
//! before the log is paged, it lets the formatter name each column's
//! values as it would name the monitored property itself: `offnormal`
//! rather than `enum 2`, `in-alarm` rather than `bits(1000)`, and a binary
//! object's state as `active`.
//!
//! Naming never fails a read. A reference that can't be read, doesn't
//! decode or names nothing leaves its column as the log gives it, and so
//! does a value whose datatype the property doesn't name (a REAL logged
//! where an enumeration was expected prints as the REAL).

use bacnet_encoding::constructed::decode_device_object_property_reference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

use super::{Datum, ValueFormat};

/// The property a log column samples, from its Log_DeviceObjectProperty
/// reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggedProperty {
    /// The monitored object's type, which names a binary or multi-state
    /// object's states.
    pub object_type: ObjectType,
    /// The monitored property.
    pub property: PropertyIdentifier,
    /// The array index the reference names, if any.
    pub index: Option<u32>,
}

/// What each column of a log samples: one column for a Trend Log, one per
/// member for a Trend Log Multiple, `None` where the reference is empty.
/// The default names no column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogColumns(Vec<Option<LoggedProperty>>);

impl LogColumns {
    /// Whether a log of `object_type` has a Log_DeviceObjectProperty to
    /// name its values by: a Trend Log or a Trend Log Multiple.
    pub fn applies(object_type: ObjectType) -> bool {
        object_type == ObjectType::TREND_LOG || object_type == ObjectType::TREND_LOG_MULTIPLE
    }

    /// The columns a Log_DeviceObjectProperty value names, read whole:
    /// its references back to back. Octets that don't decode to the end
    /// name no column, so the values print as the log gives them.
    pub fn decode(data: &[u8]) -> Self {
        let mut columns = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let Ok((reference, next)) = decode_device_object_property_reference(data, offset)
            else {
                return Self::default();
            };
            columns.push((!reference.is_unset()).then(|| LoggedProperty {
                object_type: reference.object_identifier.object_type(),
                property: PropertyIdentifier::from_raw(reference.property_identifier),
                index: reference.property_array_index,
            }));
            offset = next;
        }
        columns.into_iter().collect()
    }

    /// The property column `column` (from 0) samples, if named.
    pub fn column(&self, column: usize) -> Option<LoggedProperty> {
        self.0.get(column).copied().flatten()
    }

    /// Whether no column is named.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }
}

impl FromIterator<Option<LoggedProperty>> for LogColumns {
    /// The columns in order, `None` for one nothing names.
    fn from_iter<I: IntoIterator<Item = Option<LoggedProperty>>>(columns: I) -> Self {
        Self(columns.into_iter().collect())
    }
}

/// The property a log's own values belong to when no column is named: one
/// with no enumeration, named bits or constructed element.
const UNNAMED: PropertyIdentifier = PropertyIdentifier::LOG_BUFFER;

impl ValueFormat<'_> {
    /// This formatter for the object `column` monitors, and the property to
    /// name its values by.
    fn for_column(&self, column: Option<LoggedProperty>) -> (Self, PropertyIdentifier) {
        match column {
            Some(column) => (
                Self {
                    object_type: column.object_type,
                    ..*self
                },
                column.property,
            ),
            None => (*self, UNNAMED),
        }
    }

    /// One logged value as text, named for the property `column` samples.
    /// An enumeration nothing names shows as `enum 3`, so it isn't taken
    /// for a count; a bit string nothing names shows as its bits.
    pub fn logged(&self, column: Option<LoggedProperty>, value: &PropertyValue) -> String {
        let (format, property) = self.for_column(column);
        let text = format.element(property, value);
        match value {
            PropertyValue::Enumerated(raw) if text == raw.to_string() => format!("enum {raw}"),
            _ => text,
        }
    }

    /// [`Self::logged`] as typed data: an enumeration with its name and a
    /// bit string with its set bits' names, when the property names them.
    pub fn logged_data(&self, column: Option<LoggedProperty>, value: &PropertyValue) -> Datum {
        let (format, property) = self.for_column(column);
        format.element_data(property, value)
    }

    /// A logged value of another datatype, as its tagged octets, as text:
    /// decoded as a read of the property `column` samples would be.
    pub fn logged_octets(&self, column: Option<LoggedProperty>, octets: &[u8]) -> String {
        let (format, property) = self.for_column(column);
        format.value_at(property, column.and_then(|column| column.index), octets)
    }

    /// [`Self::logged_octets`] as typed data.
    pub fn logged_octets_data(&self, column: Option<LoggedProperty>, octets: &[u8]) -> Datum {
        let (format, property) = self.for_column(column);
        format.data_at(property, column.and_then(|column| column.index), octets)
    }
}

#[cfg(test)]
#[path = "logged_tests.rs"]
mod tests;
