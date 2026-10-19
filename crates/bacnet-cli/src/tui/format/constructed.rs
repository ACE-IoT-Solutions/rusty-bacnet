//! Constructed elements as text: the productions the element table names,
//! split with their codecs and written out in the TUI's style.
//!
//! Only the elements a technician meets on everyday objects have a renderer
//! here; any other element, or a value that doesn't decode as its elements
//! to the last octet, is left to the generic view.

use bacnet_encoding::constructed::{
    decode_calendar_entry, decode_daily_schedule, decode_date_range,
    decode_device_object_property_reference, decode_device_object_reference, decode_name_value,
    decode_object_property_reference_at, decode_special_event, decode_value_source, split_elements,
    ConstructedElement,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDeviceObjectReference, BACnetTimeValue, BACnetValueSource,
    SpecialEventPeriod,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};

use super::primitive::{date_range, date_text, date_time, time_text, weekday};
use super::ValueFormat;
use crate::tui::app::objects::{kebab, object_label};

/// The object instance an unset reference names: the standard keeps
/// 4194303 for an object that isn't there.
const UNSET_INSTANCE: u32 = 4_194_303;

/// The text of each element of `data`, or `None` when the element has no
/// renderer, `data` is empty, or an element doesn't decode.
pub(super) fn elements(
    format: &ValueFormat<'_>,
    element: ConstructedElement,
    data: &[u8],
) -> Option<Vec<String>> {
    if data.is_empty() {
        return None;
    }
    let render = renderer(element)?;
    let mut texts = Vec::new();
    let ends = split_elements(data, |octets, offset| {
        render(format, texts.len(), octets, offset).map(|(text, end)| {
            texts.push(text);
            end
        })
    })?;
    debug_assert_eq!(ends.len(), texts.len());
    Some(texts)
}

/// Decodes the element at an offset, the `index`th (from 0) of the value,
/// and writes it out.
type Render = fn(&ValueFormat<'_>, usize, &[u8], usize) -> Result<(String, usize), Error>;

fn renderer(element: ConstructedElement) -> Option<Render> {
    use ConstructedElement as E;
    Some(match element {
        E::TimeStamp => timestamp,
        E::DailySchedule => daily_schedule,
        E::SpecialEvent => special_event,
        E::CalendarEntry => calendar_entry,
        E::DateRange => |_, _, octets, offset| {
            decode_date_range(octets, offset)
                .map(|(range, end)| (date_range(&range.start_date, &range.end_date), end))
        },
        E::DeviceObjectPropertyReference => |_, _, octets, offset| {
            decode_device_object_property_reference(octets, offset).map(|(r, end)| {
                let text = reference(
                    r.object_identifier,
                    r.property_identifier,
                    r.property_array_index,
                );
                (on_device(text, r.device_identifier), end)
            })
        },
        E::ObjectPropertyReference => |_, _, octets, offset| {
            decode_object_property_reference_at(octets, offset).map(|(r, end)| {
                let text = reference(
                    r.object_identifier,
                    r.property_identifier,
                    r.property_array_index,
                );
                (text, end)
            })
        },
        E::DeviceObjectReference => |_, _, octets, offset| {
            decode_device_object_reference(octets, offset)
                .map(|(r, end)| (object_reference(&r), end))
        },
        E::ValueSource => |_, _, octets, offset| {
            decode_value_source(octets, offset).map(|(source, end)| {
                let text = match source {
                    BACnetValueSource::None => "none".to_string(),
                    BACnetValueSource::Object(r) => object_reference(&r),
                    BACnetValueSource::Address(address) => format!(
                        "network {} address 0x{}",
                        address.network_number,
                        address
                            .mac_address
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    ),
                };
                (text, end)
            })
        },
        E::NameValue => |format, _, octets, offset| {
            decode_name_value(octets, offset).map(|(tag, end)| {
                let text = match &tag.value {
                    Some(value) => format!(
                        "{} = {}",
                        tag.name,
                        format.element(PropertyIdentifier::TAGS, value)
                    ),
                    None => tag.name.clone(),
                };
                (text, end)
            })
        },
        _ => return None,
    })
}

fn timestamp(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_timestamp_choice(octets, offset).map(|(stamp, end)| {
        let text = match stamp {
            BACnetTimeStamp::Time(time) => time_text(&time),
            BACnetTimeStamp::SequenceNumber(n) => format!("seq {n}"),
            BACnetTimeStamp::DateTime { date, time } => date_time(&date, &time),
        };
        (text, end)
    })
}

/// Time and value pairs: `07:00 = active, 18:00 = inactive`.
fn time_values(format: &ValueFormat<'_>, values: &[BACnetTimeValue]) -> String {
    if values.is_empty() {
        return "none".into();
    }
    values
        .iter()
        .map(|tv| {
            format!(
                "{} = {}",
                time_text(&tv.time),
                format.element(PropertyIdentifier::PRESENT_VALUE, &tv.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One day of a Weekly_Schedule, named for its place in the week, which
/// starts on Monday (Clause 12.24).
fn daily_schedule(
    format: &ValueFormat<'_>,
    index: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_daily_schedule(octets, offset).map(|(values, end)| {
        let label = u8::try_from(index + 1)
            .ok()
            .and_then(weekday)
            .unwrap_or("day");
        (format!("{label} {}", time_values(format, &values)), end)
    })
}

fn calendar_text(entry: &BACnetCalendarEntry) -> String {
    match entry {
        BACnetCalendarEntry::Date(date) => date_text(date),
        BACnetCalendarEntry::DateRange(range) => date_range(&range.start_date, &range.end_date),
        BACnetCalendarEntry::WeekNDay(week) => {
            let field = |value: u8| {
                if value == 0xFF {
                    "*".to_string()
                } else {
                    value.to_string()
                }
            };
            format!(
                "month {} week {} {}",
                field(week.month),
                field(week.week_of_month),
                weekday(week.day_of_week).unwrap_or("any day")
            )
        }
    }
}

fn calendar_entry(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_calendar_entry(octets, offset).map(|(entry, end)| (calendar_text(&entry), end))
}

fn special_event(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_special_event(octets, offset).map(|(event, end)| {
        let period = match &event.period {
            SpecialEventPeriod::CalendarEntry(entry) => calendar_text(entry),
            SpecialEventPeriod::CalendarReference(calendar) => object_label(*calendar),
        };
        let text = format!(
            "{period}: {} (priority {})",
            time_values(format, &event.list_of_time_values),
            event.event_priority
        );
        (text, end)
    })
}

/// `analog-input 2 present-value`, with `[3]` for an array index; `none`
/// for the unset reference.
fn reference(object: ObjectIdentifier, property: u32, index: Option<u32>) -> String {
    if object.instance_number() == UNSET_INSTANCE {
        return "none".into();
    }
    let mut text = format!(
        "{} {}",
        object_label(object),
        kebab(PropertyIdentifier::from_raw(property))
    );
    if let Some(index) = index {
        text.push_str(&format!("[{index}]"));
    }
    text
}

fn on_device(text: String, device: Option<ObjectIdentifier>) -> String {
    match device {
        Some(device) if text != "none" => format!("{text} on device {}", device.instance_number()),
        _ => text,
    }
}

fn object_reference(reference: &BACnetDeviceObjectReference) -> String {
    let object = if reference.object_identifier.instance_number() == UNSET_INSTANCE {
        "none".to_string()
    } else {
        object_label(reference.object_identifier)
    };
    on_device(object, reference.device_identifier)
}
