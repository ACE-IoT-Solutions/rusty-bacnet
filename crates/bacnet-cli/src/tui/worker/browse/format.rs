//! Turn property values as read into the browser's display data.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{resolve_bits, resolve_enum, PropertyIdentifier, ResolvedBits};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::tui::point::{
    DeviceCard, Health, ObjectView, PriorityView, PropertyRow, ReadVia, PRIORITIES,
};

/// A property as read: its raw application-tagged value, or an error.
pub(crate) type RawRow = (PropertyIdentifier, Result<Vec<u8>, String>);

/// Values shown before the rest are summarised.
const MAX_LIST_SHOWN: usize = 8;

/// Every application-tagged value in `data`, or `None` if any part of it
/// is not one (a constructed value).
pub(crate) fn decode_all(data: &[u8]) -> Option<Vec<PropertyValue>> {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        let (value, next) = decode_application_value(data, offset).ok()?;
        values.push(value);
        offset = next;
    }
    Some(values)
}

/// One value, with enumerations and bit strings named for `property`.
fn one(property: PropertyIdentifier, value: &PropertyValue) -> String {
    if let Some(named) = resolve_enum(property, value) {
        return named.to_string();
    }
    match resolve_bits(property, value) {
        Some(ResolvedBits::Unknown { .. }) | None => crate::output::format_property_value(value),
        Some(bits) => bits.to_string(),
    }
}

/// A property's value as display text.
pub(crate) fn value_text(property: PropertyIdentifier, data: &[u8]) -> String {
    let Some(values) = decode_all(data) else {
        let hex: Vec<String> = data.iter().map(|b| format!("{b:02x}")).collect();
        return format!("[raw {}]", hex.join(" "));
    };
    match values.as_slice() {
        [] => "(empty)".into(),
        [value] => one(property, value),
        many => {
            let shown: Vec<String> = many
                .iter()
                .take(MAX_LIST_SHOWN)
                .map(|value| one(property, value))
                .collect();
            let more = many.len().saturating_sub(MAX_LIST_SHOWN);
            if more == 0 {
                format!("[{}]", shown.join(", "))
            } else {
                format!(
                    "[{}, +{more} more] ({} items)",
                    shown.join(", "),
                    many.len()
                )
            }
        }
    }
}

/// The single value of `property` among `rows`.
fn single(rows: &[RawRow], property: PropertyIdentifier) -> Option<PropertyValue> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    let mut values = decode_all(value.as_ref().ok()?)?;
    (values.len() == 1).then(|| values.remove(0))
}

/// How many bits of a supported-services or object-types bit string are set.
fn supported(data: &[u8], property: PropertyIdentifier) -> Option<String> {
    let value = decode_all(data)?.into_iter().next()?;
    let count = match resolve_bits(property, &value)? {
        ResolvedBits::ServicesSupported(bits) => bits.iter().count(),
        ResolvedBits::ObjectTypesSupported(bits) => bits.iter().count(),
        _ => return None,
    };
    Some(format!("{count} supported"))
}

/// The device card from the Device object's properties.
pub(crate) fn card(rows: &[RawRow], via: ReadVia) -> DeviceCard {
    const LABELS: [(PropertyIdentifier, &str); 13] = [
        (PropertyIdentifier::OBJECT_NAME, "Name"),
        (PropertyIdentifier::VENDOR_NAME, "Vendor"),
        (PropertyIdentifier::VENDOR_IDENTIFIER, "Vendor ID"),
        (PropertyIdentifier::MODEL_NAME, "Model"),
        (PropertyIdentifier::FIRMWARE_REVISION, "Firmware"),
        (PropertyIdentifier::APPLICATION_SOFTWARE_VERSION, "Software"),
        (PropertyIdentifier::PROTOCOL_VERSION, "Protocol version"),
        (PropertyIdentifier::PROTOCOL_REVISION, "Protocol revision"),
        (PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, "Services"),
        (
            PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
            "Object types",
        ),
        (PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, "Max APDU"),
        (PropertyIdentifier::SEGMENTATION_SUPPORTED, "Segmentation"),
        (PropertyIdentifier::DATABASE_REVISION, "Database revision"),
    ];
    let rows = LABELS
        .iter()
        .map(|&(property, label)| {
            let text = match rows.iter().find(|(p, _)| *p == property) {
                Some((_, Ok(data))) => {
                    supported(data, property).unwrap_or_else(|| value_text(property, data))
                }
                Some((_, Err(error))) => format!("({error})"),
                None => "-".into(),
            };
            (label, text)
        })
        .collect();
    DeviceCard {
        rows,
        via: Some(via),
    }
}

/// The health badges: Status_Flags as the device reports them, or, for an
/// object without it, what Event_State, Reliability and Out_Of_Service say
/// (Clause 12 defines each flag from those properties).
fn health(rows: &[RawRow]) -> Health {
    if let Some(PropertyValue::BitString { data, .. }) =
        single(rows, PropertyIdentifier::STATUS_FLAGS)
    {
        let flags = bacnet_types::bitstring::status_flags_from_bacnet(&data);
        return Health {
            alarm: flags.contains(StatusFlags::IN_ALARM),
            fault: flags.contains(StatusFlags::FAULT),
            overridden: flags.contains(StatusFlags::OVERRIDDEN),
            out_of_service: flags.contains(StatusFlags::OUT_OF_SERVICE),
        };
    }
    let nonzero =
        |property| matches!(single(rows, property), Some(PropertyValue::Enumerated(n)) if n != 0);
    Health {
        alarm: nonzero(PropertyIdentifier::EVENT_STATE),
        fault: nonzero(PropertyIdentifier::RELIABILITY),
        overridden: false,
        out_of_service: matches!(
            single(rows, PropertyIdentifier::OUT_OF_SERVICE),
            Some(PropertyValue::Boolean(true))
        ),
    }
}

/// The priority pane, for an object with a Priority_Array of 16 values.
fn priority(rows: &[RawRow]) -> Option<PriorityView> {
    let (_, data) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRIORITY_ARRAY)?;
    let values = decode_all(data.as_ref().ok()?)?;
    if values.len() != PRIORITIES {
        return None;
    }
    let mut view = PriorityView::default();
    for (slot, value) in view.slots.iter_mut().zip(&values) {
        if *value != PropertyValue::Null {
            *slot = Some(one(PropertyIdentifier::PRESENT_VALUE, value));
        }
    }
    let text = |property| {
        rows.iter()
            .find(|(p, _)| *p == property)
            .and_then(|(_, value)| value.as_ref().ok())
            .map(|data| value_text(property, data))
    };
    view.relinquish_default = text(PropertyIdentifier::RELINQUISH_DEFAULT);
    view.present_value = text(PropertyIdentifier::PRESENT_VALUE);
    Some(view)
}

/// One object's view from its properties.
pub(crate) fn object_view(object: ObjectIdentifier, rows: &[RawRow], via: ReadVia) -> ObjectView {
    let priority = priority(rows);
    let properties = rows
        .iter()
        .map(|(property, value)| PropertyRow {
            property: *property,
            value: match value {
                Ok(_) if *property == PropertyIdentifier::PRIORITY_ARRAY && priority.is_some() => {
                    Ok(match priority.as_ref().and_then(PriorityView::winner) {
                        Some(slot) => format!("priority {slot} in control"),
                        None => "all relinquished".into(),
                    })
                }
                Ok(data) => Ok(value_text(*property, data)),
                Err(error) => Err(error.clone()),
            },
        })
        .collect();
    ObjectView {
        object,
        properties,
        health: health(rows),
        priority,
        via,
    }
}

#[cfg(test)]
mod tests {
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    use super::*;

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for value in values {
            encode_property_value(&mut buf, value).unwrap();
        }
        buf.to_vec()
    }

    #[test]
    fn health_comes_from_status_flags_or_the_properties_behind_them() {
        let flags = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0101_0000],
        };
        let rows = vec![(PropertyIdentifier::STATUS_FLAGS, Ok(encoded(&[flags])))];
        let flagged = health(&rows);
        assert!(flagged.fault && flagged.out_of_service && !flagged.alarm && !flagged.overridden);
        // Without Status_Flags, the properties behind them.
        let rows = vec![
            (
                PropertyIdentifier::EVENT_STATE,
                Ok(encoded(&[PropertyValue::Enumerated(2)])),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                Ok(encoded(&[PropertyValue::Boolean(true)])),
            ),
        ];
        assert_eq!(health(&rows).badges(), ["[ALM]", "[OOS]"]);
    }

    #[test]
    fn a_priority_array_needs_sixteen_values() {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[9] = PropertyValue::Real(55.0);
        let rows = vec![
            (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&[PropertyValue::Real(0.0)])),
            ),
        ];
        let view = priority(&rows).unwrap();
        assert_eq!(view.winner(), Some(10));
        assert_eq!(view.slots[9].as_deref(), Some("55"));
        assert_eq!(view.relinquish_default.as_deref(), Some("0"));
        let short = vec![(PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots[..4])))];
        assert!(priority(&short).is_none());
    }

    #[test]
    fn values_name_their_enumerations_and_summarise_long_lists() {
        assert_eq!(
            value_text(
                PropertyIdentifier::EVENT_STATE,
                &encoded(&[PropertyValue::Enumerated(1)])
            ),
            "FAULT"
        );
        let many: Vec<_> = (0..10).map(PropertyValue::Unsigned).collect();
        assert_eq!(
            value_text(PropertyIdentifier::DESCRIPTION, &encoded(&many)),
            "[0, 1, 2, 3, 4, 5, 6, 7, +2 more] (10 items)"
        );
        assert_eq!(
            value_text(PropertyIdentifier::DESCRIPTION, &[0xFF]),
            "[raw ff]"
        );
    }
}
