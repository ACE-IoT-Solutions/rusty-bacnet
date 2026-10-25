//! Notification and subscription elements as text: Recipient_List
//! destinations, recipients, COV subscriptions, address bindings and an
//! Event Enrollment's Event_Parameters.

use bacnet_encoding::constructed::{
    decode_cov_subscription, decode_destination, decode_event_parameter, decode_recipient,
};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVSubscription, BACnetDestination, BACnetEventParameter,
    BACnetPropertyStates, BACnetRecipient, ChangeOfValueCriteria,
};
use bacnet_types::enums::{
    AccessEvent, AccessZoneOccupancyState, BinaryPV, DeviceStatus, DoorAlarmState,
    DoorSecuredStatus, DoorStatus, EngineeringUnits, EventState, EventType, FileAccessMethod,
    LifeSafetyMode, LifeSafetyOperation, LifeSafetyState, LockStatus, Maintenance, NodeType,
    NotifyType, Polarity, ProgramChange, ProgramError, ProgramState, Reliability, RestartReason,
    SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::ValueFormat;
use super::constructed::reference;
use super::primitive;
use super::primitive::{hex, plain_bits, time_text, weekday};
use super::tagged::fields_text;
use crate::label::kebab;
use bacnet_encoding::tags::decode_tag;
use bacnet_types::enums::PropertyIdentifier;

/// The choice tag `decode_event_parameter` gives the forms that aren't the
/// wire CHOICE (an application octet string, and a legacy wrapper); those
/// are left to the generic view.
const NOT_A_CHOICE: u8 = u8::MAX;

/// The Event_Parameters alternative for no algorithm, `none`.
const NONE_ALGORITHM: u8 = 20;

/// `network 5 address 0x0a000001bac0`; an empty MAC, a broadcast on
/// that network, as `network 5 broadcast`.
pub(super) fn address_text(address: &BACnetAddress) -> String {
    network_mac(address.network_number, &address.mac_address)
}

/// A network number and MAC, as [`address_text`] writes them.
fn network_mac(network: u16, mac: &[u8]) -> String {
    if mac.is_empty() {
        format!("network {network} broadcast")
    } else {
        format!("network {network} address 0x{}", hex(mac, ""))
    }
}

/// A recipient: `device 1234`, or its address.
fn recipient_text(recipient: &BACnetRecipient) -> String {
    match recipient {
        BACnetRecipient::Device(device) => format!("device {}", device.instance_number()),
        BACnetRecipient::Address(address) => address_text(address),
    }
}

/// The days of a mask as runs: `Mon-Fri`, `Mon, Wed, Sat-Sun`, `every day`
/// or `no days`.
fn days_text(days: DaysOfWeek) -> String {
    let set: Vec<bool> = (0..7).map(|day| days.bits() & (1 << day) != 0).collect();
    match set.iter().filter(|on| **on).count() {
        0 => return "no days".into(),
        7 => return "every day".into(),
        _ => {}
    }
    let name = |day: usize| weekday(day as u8 + 1).unwrap_or("?");
    let mut runs = Vec::new();
    let mut day = 0;
    while day < 7 {
        if !set[day] {
            day += 1;
            continue;
        }
        let start = day;
        while day + 1 < 7 && set[day + 1] {
            day += 1;
        }
        runs.push(if day == start {
            name(start).to_string()
        } else {
            format!("{}-{}", name(start), name(day))
        });
        day += 1;
    }
    runs.join(", ")
}

/// Event transitions by name, `no transitions` when none is set.
fn transitions_text(transitions: EventTransitionBits) -> String {
    if transitions.is_empty() {
        "no transitions".into()
    } else {
        kebab(transitions)
    }
}

fn confirmed_text(confirmed: bool) -> &'static str {
    if confirmed {
        "confirmed"
    } else {
        "unconfirmed"
    }
}

/// One Recipient_List destination: `device 1234 process 7: Mon-Fri
/// 08:00-17:00, confirmed, to-offnormal | to-fault | to-normal`.
fn destination_text(destination: &BACnetDestination) -> String {
    format!(
        "{} process {}: {} {}-{}, {}, {}",
        recipient_text(&destination.recipient),
        destination.process_identifier,
        days_text(destination.valid_days),
        time_text(&destination.from_time),
        time_text(&destination.to_time),
        confirmed_text(destination.issue_confirmed_notifications),
        transitions_text(destination.transitions),
    )
}

pub(super) fn destination(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_destination(octets, offset).map(|(d, end)| (destination_text(&d), end))
}

pub(super) fn recipient(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_recipient(octets, offset).map(|(r, end)| (recipient_text(&r), end))
}

/// One Active_COV_Subscriptions element: `device 1234 process 7:
/// analog-input 1 present-value, unconfirmed, 120 s left, increment 0.5`.
fn cov_subscription_text(format: &ValueFormat<'_>, subscription: &BACnetCOVSubscription) -> String {
    let monitored = &subscription.monitored_property_reference;
    let mut text = format!(
        "{} process {}: {}, {}, ",
        recipient_text(&subscription.recipient.recipient),
        subscription.recipient.process_identifier,
        reference(
            monitored.object_identifier,
            monitored.property_identifier,
            monitored.property_array_index
        ),
        confirmed_text(subscription.issue_confirmed_notifications),
    );
    match subscription.time_remaining {
        0 => text.push_str("no expiry"),
        seconds => text.push_str(&format!("{seconds} s left")),
    }
    if let Some(increment) = subscription.cov_increment {
        text.push_str(&format!(", increment {}", format.real(increment)));
    }
    text
}

pub(super) fn cov_subscription(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_cov_subscription(octets, offset).map(|(s, end)| (cov_subscription_text(format, &s), end))
}

/// One Device_Address_Binding element, the Device's identifier then its
/// network number and MAC, each application tagged: `device 1234 at
/// network 0 address 0x0a000001bac0`.
pub(super) fn address_binding(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    let (device, network, mac, end) = decode_address_binding(octets, offset)?;
    let text = format!(
        "device {} at {}",
        device.instance_number(),
        network_mac(network, &mac)
    );
    Ok((text, end))
}

/// The Device, network number and MAC of the binding at `offset`, and the
/// offset past it.
fn decode_address_binding(
    octets: &[u8],
    offset: usize,
) -> Result<(ObjectIdentifier, u16, Vec<u8>, usize), Error> {
    let (PropertyValue::ObjectIdentifier(device), at) = decode_application_value(octets, offset)?
    else {
        return Err(Error::decoding(offset, "expected the binding's Device"));
    };
    let (PropertyValue::Unsigned(network), mac_at) = decode_application_value(octets, at)? else {
        return Err(Error::decoding(at, "expected the binding's network number"));
    };
    let network = u16::try_from(network)
        .map_err(|_| Error::decoding(at, "the binding's network number is over 65535"))?;
    let (PropertyValue::OctetString(mac), end) = decode_application_value(octets, mac_at)? else {
        return Err(Error::decoding(mac_at, "expected the binding's MAC"));
    };
    Ok((device, network, mac, end))
}

/// A property state by name where its enumeration is known: `high-limit`,
/// `active`, `degrees-fahrenheit`; any other as its choice and value.
pub(super) fn property_state_text(state: &BACnetPropertyStates) -> String {
    use BACnetPropertyStates as S;
    match state {
        S::BooleanValue(value) => value.to_string(),
        S::UnsignedValue(value) => value.to_string(),
        S::IntegerValue(value) => value.to_string(),
        S::BinaryValue(v) => kebab(BinaryPV::from_raw(*v)),
        S::EventType(v) => kebab(EventType::from_raw(*v)),
        S::Polarity(v) => kebab(Polarity::from_raw(*v)),
        S::ProgramChange(v) => kebab(ProgramChange::from_raw(*v)),
        S::ProgramState(v) => kebab(ProgramState::from_raw(*v)),
        S::ReasonForHalt(v) => kebab(ProgramError::from_raw(*v)),
        S::Reliability(v) => kebab(Reliability::from_raw(*v)),
        S::State(v) => kebab(EventState::from_raw(*v)),
        S::SystemStatus(v) => kebab(DeviceStatus::from_raw(*v)),
        S::Units(v) => kebab(EngineeringUnits::from_raw(*v)),
        S::LifeSafetyMode(v) => kebab(LifeSafetyMode::from_raw(*v)),
        S::LifeSafetyState(v) => kebab(LifeSafetyState::from_raw(*v)),
        S::RestartReason(v) => kebab(RestartReason::from_raw(*v)),
        S::DoorAlarmState(v) => kebab(DoorAlarmState::from_raw(*v)),
        S::DoorSecuredStatus(v) => kebab(DoorSecuredStatus::from_raw(*v)),
        S::DoorStatus(v) => kebab(DoorStatus::from_raw(*v)),
        S::FileAccessMethod(v) => kebab(FileAccessMethod::from_raw(*v)),
        S::LockStatus(v) => kebab(LockStatus::from_raw(*v)),
        S::LifeSafetyOperation(v) => kebab(LifeSafetyOperation::from_raw(*v)),
        S::Maintenance(v) => kebab(Maintenance::from_raw(*v)),
        S::NodeType(v) => kebab(NodeType::from_raw(*v)),
        S::NotifyType(v) => kebab(NotifyType::from_raw(*v)),
        S::SilencedState(v) => kebab(SilencedState::from_raw(*v)),
        S::AccessEvent(v) => kebab(AccessEvent::from_raw(*v)),
        S::ZoneOccupancyState(v) => kebab(AccessZoneOccupancyState::from_raw(*v)),
        other => match other.as_u32() {
            Some(value) => format!("{} {value}", choice_name(other)),
            None => choice_name(other),
        },
    }
}

/// The name of a property state's choice, `shed-state` for `ShedState(3)`.
fn choice_name(state: &BACnetPropertyStates) -> String {
    let debug = format!("{state:?}");
    let name = debug.split('(').next().unwrap_or_default();
    let mut text = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            text.push('-');
        }
        text.push(ch.to_ascii_lowercase());
    }
    text
}

/// The algorithm of an Event_Parameters choice, named as the event type
/// with the same number.
fn algorithm(tag: u8) -> String {
    kebab(EventType::from_raw(u32::from(tag)))
}

fn delay(seconds: u32) -> String {
    format!("delay {seconds} s")
}

/// Bit string octets as `bits(0110)`.
fn bit_text(unused_bits: u8, data: &[u8]) -> String {
    plain_bits(&PropertyValue::BitString {
        unused_bits,
        data: data.to_vec(),
    })
}

/// An Event Enrollment's Event_Parameters: the algorithm, then its
/// parameters, `out-of-range: delay 5 s, low 10, high 90, deadband 1`.
fn event_parameter_text(format: &ValueFormat<'_>, parameter: &BACnetEventParameter) -> String {
    let name = algorithm(parameter.tag());
    let real = |value: f32| format.real(value);
    let details = match parameter {
        BACnetEventParameter::ChangeOfBitstring {
            time_delay,
            bitmask,
            list_of_values,
        } => format!(
            "{}, mask {}, alarm values [{}]",
            delay(*time_delay),
            bit_text(bitmask.0, &bitmask.1),
            list_of_values
                .iter()
                .map(|(unused, data)| bit_text(*unused, data))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        BACnetEventParameter::ChangeOfState {
            time_delay,
            list_of_values,
        } => format!(
            "{}, alarm values [{}]",
            delay(*time_delay),
            list_of_values
                .iter()
                .map(property_state_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        BACnetEventParameter::ChangeOfValue {
            time_delay,
            criteria,
        } => {
            let criteria = match criteria {
                ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                    format!("mask {}", bit_text(*unused_bits, data))
                }
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment) => {
                    format!("increment {}", real(*increment))
                }
            };
            format!("{}, {criteria}", delay(*time_delay))
        }
        BACnetEventParameter::FloatingLimit {
            time_delay,
            setpoint_reference: r,
            low_diff_limit,
            high_diff_limit,
            deadband,
        } => {
            let mut setpoint = reference(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            );
            if let Some(device) = r.device_identifier {
                setpoint.push_str(&format!(" on device {}", device.instance_number()));
            }
            format!(
                "{}, setpoint {setpoint}, low diff {}, high diff {}, deadband {}",
                delay(*time_delay),
                real(*low_diff_limit),
                real(*high_diff_limit),
                real(*deadband)
            )
        }
        BACnetEventParameter::OutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => format!(
            "{}, low {}, high {}, deadband {}",
            delay(*time_delay),
            real(*low_limit),
            real(*high_limit),
            real(*deadband)
        ),
        BACnetEventParameter::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => format!(
            "vendor {vendor_id} type {extended_event_type}, parameters ({})",
            fields_text(parameters, format.floats)
        ),
        BACnetEventParameter::Opaque { tag, data } if *tag == NONE_ALGORITHM => {
            return if data.is_empty() {
                "none".into()
            } else {
                format!("none {}", fields_text(data, format.floats))
            };
        }
        // An algorithm with no model here: the out-of-range forms by their
        // fields, any other by context tag.
        BACnetEventParameter::Opaque { tag, data } => {
            out_of_range(format, *tag, data).unwrap_or_else(|| fields_text(data, format.floats))
        }
    };
    format!("{name}: {details}")
}

/// The datatype of an out-of-range limit.
#[derive(Clone, Copy)]
enum Limit {
    Unsigned,
    Signed,
    Double,
}

/// The double-, signed- and unsigned-out-of-range parameters (choices 14,
/// 15 and 16): a time delay, then low limit `[1]`, high limit `[2]` and
/// deadband `[3]`, the limits of the choice's datatype and the deadband a
/// Double for the double form and an Unsigned otherwise. `None` for any
/// other choice, or a body of another shape.
fn out_of_range(format: &ValueFormat<'_>, tag: u8, data: &[u8]) -> Option<String> {
    let (limits, deadband) = match tag {
        14 => (Limit::Double, Limit::Double),
        15 => (Limit::Signed, Limit::Unsigned),
        16 => (Limit::Unsigned, Limit::Unsigned),
        _ => return None,
    };
    let mut offset = 0;
    let mut field = |number: u8, kind: Limit| -> Option<String> {
        let (tag, contents) = decode_tag(data, offset).ok()?;
        if !tag.is_context(number) || tag.is_opening || tag.is_closing {
            return None;
        }
        let octets = data.get(contents..contents.checked_add(tag.length as usize)?)?;
        offset = contents + octets.len();
        let text = match kind {
            Limit::Unsigned if (1..=8).contains(&octets.len()) => octets
                .iter()
                .fold(0u64, |n, b| (n << 8) | u64::from(*b))
                .to_string(),
            Limit::Signed if (1..=8).contains(&octets.len()) => {
                let negative = octets[0] & 0x80 != 0;
                let start = if negative { -1i64 } else { 0 };
                octets
                    .iter()
                    .fold(start, |n, b| (n << 8) | i64::from(*b))
                    .to_string()
            }
            Limit::Double => {
                let value = f64::from_be_bytes(octets.try_into().ok()?);
                primitive::text(
                    PropertyIdentifier::PRESENT_VALUE,
                    &PropertyValue::Double(value),
                    format.floats,
                )
            }
            _ => return None,
        };
        Some(text)
    };
    let delay_seconds = field(0, Limit::Unsigned)?;
    let low = field(1, limits)?;
    let high = field(2, limits)?;
    let band = field(3, deadband)?;
    if offset != data.len() {
        return None;
    }
    Some(format!(
        "delay {delay_seconds} s, low {low}, high {high}, deadband {band}"
    ))
}

pub(super) fn event_parameter(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    let (parameter, end) = decode_event_parameter(octets, offset)?;
    if let BACnetEventParameter::Opaque {
        tag: NOT_A_CHOICE, ..
    } = parameter
    {
        return Err(Error::decoding(offset, "not an Event_Parameters choice"));
    }
    Ok((event_parameter_text(format, &parameter), end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_written_as_runs() {
        let days = |bits| days_text(DaysOfWeek::from_bits_truncate(bits));
        assert_eq!(days(0b001_1111), "Mon-Fri");
        assert_eq!(days(0b111_1111), "every day");
        assert_eq!(days(0), "no days");
        assert_eq!(days(0b110_0101), "Mon, Wed, Sat-Sun");
        assert_eq!(days(0b000_0010), "Tue");
    }

    #[test]
    fn property_states_without_a_named_enumeration_keep_their_choice() {
        assert_eq!(
            property_state_text(&BACnetPropertyStates::ShedState(3)),
            "shed-state 3"
        );
        assert_eq!(
            property_state_text(&BACnetPropertyStates::State(3)),
            "high-limit"
        );
    }
}
