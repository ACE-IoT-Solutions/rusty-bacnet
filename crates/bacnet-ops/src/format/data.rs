//! Values as typed data, for JSON: numbers as numbers, enumerations with
//! their number and name, bit strings with their bits and set names, and
//! constructed elements as the text the formatter writes for them.
//!
//! The shape is a contract of the CLI's JSON output (see the CLI
//! reference, `docs/CLI.md`, "Value JSON"); names are kebab case, as in the
//! text.

use bacnet_encoding::constructed::ElementShape;
use bacnet_types::enums::{
    PropertyIdentifier, ResolvedBits, ResolvedEnum, resolve_bits, resolve_enum,
};
use bacnet_types::primitives::PropertyValue;
use serde::Serialize;

use super::primitive::{date_text, hex, plain_bits, time_text};
use super::{ValueFormat, constructed, decode_all, holds_state, is_binary};
use crate::label::kebab;

/// A property value as typed data. Serialized untagged, so each form is
/// told apart by its JSON type and keys.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Datum {
    /// NULL, as `null`.
    Null,
    /// A BOOLEAN, as `true` or `false`.
    Boolean(bool),
    /// An Unsigned, as a number.
    Unsigned(u64),
    /// A Signed, as a number.
    Signed(i64),
    /// A REAL, as a number.
    Real(f32),
    /// A Double, as a number.
    Double(f64),
    /// A REAL or Double that is not a number or infinite, which JSON
    /// numbers can't hold: `{"float": "NaN"}`.
    NonFinite {
        /// `NaN`, `inf` or `-inf`.
        float: String,
    },
    /// A CharacterString, as a string.
    Text(String),
    /// An OCTET STRING: `{"octets": "0a0b"}`, lowercase hex.
    Octets {
        /// The octets as hex.
        octets: String,
    },
    /// An ENUMERATED: `{"value": 2, "name": "offnormal"}`, without a name
    /// when the property's enumeration isn't known.
    Enumerated {
        /// The number on the wire.
        value: u32,
        /// Its name in kebab case.
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// A BIT STRING: `{"bits": "0100", "names": ["fault"]}`, first bit
    /// first. `names` lists the set bits by name, and is left out when the
    /// property's bits have no names.
    Bits {
        /// One digit a bit.
        bits: String,
        /// The set bits' names in kebab case.
        #[serde(skip_serializing_if = "Option::is_none")]
        names: Option<Vec<String>>,
    },
    /// A Date: `{"date": "2026-10-19"}`, in the text's form.
    Date {
        /// The date as the text writes it.
        date: String,
    },
    /// A Time: `{"time": "07:00"}`, in the text's form.
    Time {
        /// The time as the text writes it.
        time: String,
    },
    /// A BACnetObjectIdentifier: `{"type": "analog-input", "type_number":
    /// 0, "instance": 1}`.
    Object {
        /// The object type in kebab case.
        #[serde(rename = "type")]
        object_type: String,
        /// The object type's number.
        type_number: u32,
        /// The instance number.
        instance: u32,
    },
    /// Several values, or the elements of a list or array, as an array.
    List(Vec<Datum>),
    /// One constructed element (a timestamp, a schedule day, a recipient):
    /// `{"element": "daily-schedule", "text": "Mon 07:00 = active"}`. The
    /// element names its kind, so structured fields can join it later; the
    /// text is for display, not for parsing.
    Constructed {
        /// The element's kind in kebab case, `destination`.
        element: String,
        /// The element as the text writes it.
        text: String,
    },
    /// Octets that don't decode: `{"raw": "0e0f"}`.
    Raw {
        /// The octets as hex.
        raw: String,
    },
}

impl ValueFormat<'_> {
    /// A property's value as typed data, decoded as [`Self::value`] decodes
    /// it: a constructed value element by element, and otherwise as
    /// application values, one a datum and several a list.
    pub fn data(&self, property: PropertyIdentifier, data: &[u8]) -> Datum {
        self.data_at(property, None, data)
    }

    /// [`Self::data`] of a read at an array index, as
    /// [`Self::value_at`] reads it.
    pub fn data_at(&self, property: PropertyIdentifier, index: Option<u32>, data: &[u8]) -> Datum {
        if let Some((element, shape, texts)) = self.elements_at(property, index, data) {
            let kind = constructed::element_name(element);
            let mut items: Vec<Datum> = texts
                .into_iter()
                .map(|text| Datum::Constructed {
                    element: kind.into(),
                    text,
                })
                .collect();
            return match shape {
                ElementShape::Single if items.len() == 1 => items.remove(0),
                _ => Datum::List(items),
            };
        }
        let Some(values) = decode_all(data) else {
            return Datum::Raw { raw: hex(data, "") };
        };
        match values.as_slice() {
            [one] => self.element_data(property, one),
            many => Datum::List(
                many.iter()
                    .map(|value| self.element_data(property, value))
                    .collect(),
            ),
        }
    }

    /// One application value of `property` as typed data.
    pub fn element_data(&self, property: PropertyIdentifier, value: &PropertyValue) -> Datum {
        match value {
            PropertyValue::Null => Datum::Null,
            PropertyValue::Boolean(b) => Datum::Boolean(*b),
            PropertyValue::Unsigned(n) => Datum::Unsigned(*n),
            PropertyValue::Signed(n) => Datum::Signed(i64::from(*n)),
            PropertyValue::Real(v) if v.is_finite() => Datum::Real(*v),
            PropertyValue::Double(v) if v.is_finite() => Datum::Double(*v),
            PropertyValue::Real(v) => non_finite(f64::from(*v)),
            PropertyValue::Double(v) => non_finite(*v),
            PropertyValue::CharacterString(text) => Datum::Text(text.clone()),
            PropertyValue::OctetString(bytes) => Datum::Octets {
                octets: hex(bytes, ""),
            },
            PropertyValue::Enumerated(raw) => Datum::Enumerated {
                value: *raw,
                name: self.enum_name(property, value),
            },
            PropertyValue::BitString { .. } => Datum::Bits {
                bits: bit_digits(value),
                names: bit_names(property, value),
            },
            PropertyValue::Date(date) => Datum::Date {
                date: date_text(date),
            },
            PropertyValue::Time(time) => Datum::Time {
                time: time_text(time),
            },
            PropertyValue::ObjectIdentifier(object) => Datum::Object {
                object_type: kebab(object.object_type()),
                type_number: object.object_type().to_raw(),
                instance: object.instance_number(),
            },
            PropertyValue::List(items) => Datum::List(
                items
                    .iter()
                    .map(|item| self.element_data(property, item))
                    .collect(),
            ),
            PropertyValue::ApplicationData(bytes) => match self.priority_date_time(property, value)
            {
                // A date-time slot reads as Present_Value's date and time.
                Some((date, time)) => Datum::List(vec![
                    Datum::Date {
                        date: date_text(&date),
                    },
                    Datum::Time {
                        time: time_text(&time),
                    },
                ]),
                None => Datum::Raw {
                    raw: hex(bytes, ""),
                },
            },
        }
    }

    /// The name of an enumerated value of `property`: a binary object's
    /// state, or the property's own enumeration.
    fn enum_name(&self, property: PropertyIdentifier, value: &PropertyValue) -> Option<String> {
        if let PropertyValue::Enumerated(raw @ (0 | 1)) = value
            && holds_state(property)
            && is_binary(self.object_type)
        {
            return Some(if *raw == 1 { "active" } else { "inactive" }.into());
        }
        match resolve_enum(property, value)? {
            ResolvedEnum::Unknown(_) => None,
            // A number outside the enumeration's names prints as itself.
            named => Some(kebab(named)).filter(|name| name.parse::<u64>().is_err()),
        }
    }
}

fn non_finite(value: f64) -> Datum {
    Datum::NonFinite {
        float: value.to_string(),
    }
}

/// A bit string's digits, first bit first: `0100`.
fn bit_digits(value: &PropertyValue) -> String {
    let text = plain_bits(value);
    text.trim_start_matches("bits(")
        .trim_end_matches(')')
        .to_string()
}

/// The names of the set bits, when `property` names its bits.
fn bit_names(property: PropertyIdentifier, value: &PropertyValue) -> Option<Vec<String>> {
    fn names<'a>(flags: impl Iterator<Item = (&'a str, impl Sized)>) -> Vec<String> {
        flags.map(|(name, _)| kebab(name)).collect()
    }
    Some(match resolve_bits(property, value)? {
        ResolvedBits::StatusFlags(flags) => names(flags.iter_names()),
        ResolvedBits::EventTransitionBits(bits) => names(bits.iter_names()),
        ResolvedBits::LimitEnable(bits) => names(bits.iter_names()),
        ResolvedBits::ServicesSupported(services) => services.iter().map(kebab).collect(),
        ResolvedBits::ObjectTypesSupported(types) => types.iter().map(kebab).collect(),
        ResolvedBits::Unknown { .. } => return None,
    })
}
