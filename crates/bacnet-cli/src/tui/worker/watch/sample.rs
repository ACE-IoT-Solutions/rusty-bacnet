//! Turn values as read or notified into watch samples.

use bacnet_client::client::ReceivedCOVNotification;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{PropertyValue, StatusFlags};
use tokio::time::Instant;

use crate::tui::point::Health;
use crate::tui::watched::{Point, Sample};
use crate::tui::worker::browse::format::{decode_all, value_text, RawRow};

/// The value as a number for the sparkline: numbers, enumerations (a
/// multi-state or binary point's state) and booleans.
fn number(data: &[u8]) -> Option<f64> {
    let values = decode_all(data)?;
    let [value] = values.as_slice() else {
        return None;
    };
    Some(match value {
        PropertyValue::Real(v) => f64::from(*v),
        PropertyValue::Double(v) => *v,
        PropertyValue::Unsigned(v) => *v as f64,
        PropertyValue::Signed(v) => f64::from(*v),
        PropertyValue::Enumerated(v) => f64::from(*v),
        PropertyValue::Boolean(v) => f64::from(u8::from(*v)),
        _ => return None,
    })
    .filter(|v| v.is_finite())
}

/// Status_Flags as the health badges.
fn flags(data: &[u8]) -> Option<Health> {
    let values = decode_all(data)?;
    let [PropertyValue::BitString { data, .. }] = values.as_slice() else {
        return None;
    };
    let flags = bacnet_types::bitstring::status_flags_from_bacnet(data);
    Some(Health {
        alarm: flags.contains(StatusFlags::IN_ALARM),
        fault: flags.contains(StatusFlags::FAULT),
        overridden: flags.contains(StatusFlags::OVERRIDDEN),
        out_of_service: flags.contains(StatusFlags::OUT_OF_SERVICE),
    })
}

/// A sample of `property` from properties as read. `None` when the read
/// didn't return it at all.
pub(crate) fn from_rows(property: PropertyIdentifier, rows: &[RawRow]) -> Option<Sample> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    let status = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::STATUS_FLAGS)
        .and_then(|(_, value)| value.as_deref().ok())
        .and_then(flags);
    Some(Sample {
        value: value
            .as_deref()
            .map(|data| value_text(property, data))
            .map_err(Clone::clone),
        number: value.as_deref().ok().and_then(number),
        flags: status,
        at: Instant::now().into_std(),
    })
}

/// The Units of `rows`, as text such as `degrees-fahrenheit`.
pub(crate) fn units(rows: &[RawRow]) -> Option<String> {
    let (_, value) = rows.iter().find(|(p, _)| *p == PropertyIdentifier::UNITS)?;
    value
        .as_deref()
        .ok()
        .map(|data| crate::tui::app::objects::kebab(value_text(PropertyIdentifier::UNITS, data)))
}

/// The sample a COV notification carries for `point`, when it is about it:
/// the same object of the same device, with the watched property among its
/// values.
pub(crate) fn from_notification(
    point: &Point,
    received: &ReceivedCOVNotification,
) -> Option<Sample> {
    let notification = &received.notification;
    if notification.monitored_object_identifier != point.object
        || notification.initiating_device_identifier.instance_number() != point.device
    {
        return None;
    }
    let rows: Vec<RawRow> = notification
        .list_of_values
        .iter()
        .filter(|value| value.property_array_index.is_none())
        .map(|value| (value.property_identifier, Ok(value.value.clone())))
        .collect();
    from_rows(point.property, &rows)
}

#[cfg(test)]
mod tests {
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    use super::*;

    fn encoded(value: &PropertyValue) -> Vec<u8> {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, value).unwrap();
        buf.to_vec()
    }

    #[test]
    fn a_sample_has_the_value_its_number_and_the_flags() {
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&PropertyValue::Real(72.5))),
            ),
            (
                PropertyIdentifier::STATUS_FLAGS,
                Ok(encoded(&PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![0b0101_0000],
                })),
            ),
        ];
        let sample = from_rows(PropertyIdentifier::PRESENT_VALUE, &rows).unwrap();
        assert_eq!(sample.value.as_deref(), Ok("72.5"));
        assert_eq!(sample.number, Some(72.5));
        assert_eq!(sample.flags.unwrap().badges(), ["[FLT]", "[OOS]"]);
        assert!(from_rows(PropertyIdentifier::DESCRIPTION, &rows).is_none());
        let text = vec![(
            PropertyIdentifier::DESCRIPTION,
            Ok(encoded(&PropertyValue::CharacterString("x".into()))),
        )];
        let sample = from_rows(PropertyIdentifier::DESCRIPTION, &text).unwrap();
        assert_eq!((sample.number, sample.flags), (None, None));
    }
}
