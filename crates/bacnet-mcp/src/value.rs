//! Property values in the two forms the tools return (design §5): the
//! display text of the shared formatter (`bacnet_ops::format`), which
//! decodes constructed values such as timestamps and schedules, and a typed
//! JSON form with enumeration names beside their numbers and bit strings
//! with their set bits named. Raw hex is only the fallback for octets that
//! don't decode.
//!
//! Display text is never device text: state texts aren't spliced in, and a
//! value holding a character string, or anything that doesn't decode as
//! application values, has its display text put in an untrusted envelope.

use bacnet_ops::format::{Floats, StateTexts, ValueFormat, decode_all};
use bacnet_ops::label::{kebab, object_label};
use bacnet_types::enums::{
    EngineeringUnits, ObjectType, PropertyIdentifier, ResolvedBits, ResolvedEnum, resolve_bits,
    resolve_enum,
};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use serde_json::{Value, json};

use crate::untrusted::envelope;

/// A property's value as `{display, typed}`: `source` names it for an
/// envelope, `cap` caps any device text, and `units` (the object's, when
/// read) follow a number in the display text.
pub(crate) fn property_value(
    object_type: ObjectType,
    property: PropertyIdentifier,
    data: &[u8],
    units: Option<EngineeringUnits>,
    source: &str,
    cap: usize,
) -> Value {
    let texts = StateTexts::default();
    let format = ValueFormat::new(object_type, &texts, Floats::Display);
    let text = format.with_units(property, data, units);
    let values = decode_all(data);
    let display = match &values {
        Some(values) if !values.iter().any(holds_text) => json!(text),
        _ => envelope(&text, source, cap),
    };
    let typed = match values.as_deref() {
        Some([one]) => typed(property, one, source, cap),
        Some(many) => json!({
            "type": "sequence",
            "items": many.iter().map(|v| typed(property, v, source, cap)).collect::<Vec<_>>(),
        }),
        None => json!({ "type": "raw", "hex": hex(data) }),
    };
    json!({ "display": display, "typed": typed })
}

/// Whether `value` holds device text, or octets that might hide some.
fn holds_text(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::CharacterString(_) | PropertyValue::ApplicationData(_) => true,
        PropertyValue::List(items) => items.iter().any(holds_text),
        _ => false,
    }
}

/// One application value in its typed form.
pub(crate) fn typed(
    property: PropertyIdentifier,
    value: &PropertyValue,
    source: &str,
    cap: usize,
) -> Value {
    match value {
        PropertyValue::Null => json!({ "type": "null" }),
        PropertyValue::Boolean(b) => json!({ "type": "boolean", "value": b }),
        PropertyValue::Unsigned(n) => json!({ "type": "unsigned", "value": n }),
        PropertyValue::Signed(n) => json!({ "type": "signed", "value": n }),
        PropertyValue::Real(v) => json!({ "type": "real", "value": number(f64::from(*v)) }),
        PropertyValue::Double(v) => json!({ "type": "double", "value": number(*v) }),
        PropertyValue::OctetString(bytes) => json!({ "type": "octet-string", "hex": hex(bytes) }),
        PropertyValue::CharacterString(text) => {
            json!({ "type": "character-string", "value": envelope(text, source, cap) })
        }
        PropertyValue::BitString { unused_bits, data } => {
            let total = (data.len() * 8).saturating_sub(usize::from(*unused_bits));
            let bits: String = (0..total)
                .map(|i| {
                    if data[i / 8] & (0x80 >> (i % 8)) != 0 {
                        '1'
                    } else {
                        '0'
                    }
                })
                .collect();
            let mut out = json!({ "type": "bit-string", "bits": bits });
            if let Some(names) = bit_names(property, value) {
                out["set"] = json!(names);
            }
            out
        }
        PropertyValue::Enumerated(raw) => {
            let mut out = json!({ "type": "enumerated", "value": raw });
            match resolve_enum(property, value) {
                Some(ResolvedEnum::Unknown(_)) | None => {}
                Some(named) => out["name"] = json!(kebab(named)),
            }
            out
        }
        PropertyValue::Date(date) => date_json(date),
        PropertyValue::Time(time) => time_json(time),
        PropertyValue::ObjectIdentifier(object) => json!({
            "type": "object-identifier",
            "object": object_text(object.object_type(), object.instance_number()),
            "label": object_label(*object),
        }),
        PropertyValue::List(items) => json!({
            "type": "list",
            "items": items.iter().map(|v| typed(property, v, source, cap)).collect::<Vec<_>>(),
        }),
        PropertyValue::ApplicationData(bytes) => json!({ "type": "raw", "hex": hex(bytes) }),
    }
}

/// The names of a bit string's set bits, for a property whose bits have
/// names.
fn bit_names(property: PropertyIdentifier, value: &PropertyValue) -> Option<Vec<String>> {
    match resolve_bits(property, value)? {
        ResolvedBits::Unknown { .. } => None,
        named => {
            let text = kebab(named);
            Some(
                text.split('|')
                    .map(str::trim)
                    // An empty set shows as `()`.
                    .filter(|name| !name.is_empty() && *name != "()")
                    .map(str::to_string)
                    .collect(),
            )
        }
    }
}

/// The text of an encoded character string, as the device sent it (to be
/// enveloped); `None` unless the octets are one character string.
pub(crate) fn text_of(data: &[u8]) -> Option<String> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::CharacterString(text)] => Some(text.clone()),
        _ => None,
    }
}

/// The text of `property` among rows as read, as [`text_of`].
pub(crate) fn row_text(
    rows: &[bacnet_ops::format::RawRow],
    property: PropertyIdentifier,
) -> Option<String> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    text_of(value.as_deref().ok()?)
}

/// An object in the tools' argument form, `analog-input:1`.
pub(crate) fn object_text(object_type: ObjectType, instance: u32) -> String {
    format!("{}:{instance}", kebab(object_type))
}

/// A float as a JSON number, or its name when it has none in JSON.
fn number(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

/// A date field, or null when unspecified.
fn field(value: u8) -> Value {
    if value == Date::UNSPECIFIED {
        Value::Null
    } else {
        json!(value)
    }
}

fn date_json(date: &Date) -> Value {
    let year = (date.year != Date::UNSPECIFIED).then(|| u16::from(date.year) + 1900);
    json!({
        "type": "date",
        "year": year,
        "month": field(date.month),
        "day": field(date.day),
        "weekday": field(date.day_of_week),
    })
}

fn time_json(time: &Time) -> Value {
    let field = |value: u8| {
        if value == Time::UNSPECIFIED {
            Value::Null
        } else {
            json!(value)
        }
    };
    json!({
        "type": "time",
        "hour": field(time.hour),
        "minute": field(time.minute),
        "second": field(time.second),
        "hundredths": field(time.hundredths),
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    fn encoded(value: &PropertyValue) -> Vec<u8> {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, value).unwrap();
        buf.to_vec()
    }

    #[test]
    fn numbers_carry_units_and_enumerations_their_names() {
        let pv = encoded(&PropertyValue::Real(72.5));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::PRESENT_VALUE,
            &pv,
            Some(EngineeringUnits::DEGREES_FAHRENHEIT),
            "device 1 analog-input:1 present-value",
            256,
        );
        assert_eq!(value["display"], json!("72.5 °F"));
        assert_eq!(value["typed"], json!({ "type": "real", "value": 72.5 }));
        let units = encoded(&PropertyValue::Enumerated(64));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::UNITS,
            &units,
            None,
            "s",
            256,
        );
        assert_eq!(
            value["typed"],
            json!({ "type": "enumerated", "value": 64, "name": "degrees-fahrenheit" })
        );
    }

    #[test]
    fn set_bits_are_named() {
        let flags = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1001_0000],
        };
        let value = typed(PropertyIdentifier::STATUS_FLAGS, &flags, "s", 256);
        assert_eq!(value["bits"], json!("1001"));
        assert_eq!(value["set"], json!(["in-alarm", "out-of-service"]));
        let none = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        };
        assert_eq!(
            typed(PropertyIdentifier::STATUS_FLAGS, &none, "s", 256)["set"],
            json!([])
        );
    }

    #[test]
    fn device_text_is_enveloped_in_both_forms() {
        let name = encoded(&PropertyValue::CharacterString(
            "Fan\u{202E} https://x".into(),
        ));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::DESCRIPTION,
            &name,
            None,
            "device 1 analog-input:1 description",
            256,
        );
        assert_eq!(value["display"]["untrusted"], json!("\"Fan hxxps://x\""));
        assert_eq!(value["typed"]["value"]["untrusted"], json!("Fan hxxps://x"));
        assert_eq!(
            value["typed"]["value"]["source"],
            json!("device 1 analog-input:1 description")
        );
    }
}
