//! Read commands: ReadProperty (RP) and ReadPropertyMultiple (RPM).
//!
//! ReadRange is in [`super::read_range`].

use bacnet_client::client::BACnetClient;
use bacnet_client::tags::{TagsRead, decode_tags_read};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_ops::destination::Destination;
use bacnet_ops::format::{Floats, ValueFormat};
use bacnet_ops::parse;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::PropertyValue;

use crate::output::{self, OutputFormat};

/// The one-shot CLI's value formatter for an object of `object_type`:
/// the shared formatter with every digit and every list element.
pub(super) fn formatter(object_type: ObjectType) -> ValueFormat<'static> {
    ValueFormat::new(object_type, &output::NO_STATE_TEXTS, Floats::Exact).whole_lists()
}

/// The items of a list or array value as text, one an element, with the
/// hex of whatever follows the last item that decoded: constructed
/// elements when the element table names the property's element, and
/// otherwise application values.
pub(super) fn format_items(
    format: &ValueFormat<'_>,
    property: PropertyIdentifier,
    data: &[u8],
) -> (Vec<String>, Option<String>) {
    if let Some(items) = format.constructed_items(property, data) {
        return (items, None);
    }
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(format.element(property, &value));
                offset = next;
            }
            Err(_) => return (values, Some(hex(&data[offset..]))),
        }
    }
    (values, None)
}

/// Octets as space-separated lowercase hex.
pub(super) fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a single property and print its value.
pub async fn read_property_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;

    // If the property is ALL, use RPM instead.
    if property == PropertyIdentifier::ALL {
        return read_all_properties(client, to, object_type, instance, format).await;
    }

    let ack = to.read_property(client, oid, property, index).await?;
    let object_type = ack.object_identifier.object_type();
    let decoded = format_read_value(
        object_type,
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    );
    let data = formatter(object_type).data_at(
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    );

    output::print_read_result(
        &format!(
            "{}:{}",
            object_type,
            ack.object_identifier.instance_number()
        ),
        &format!("{}", ack.property_identifier),
        ack.property_array_index,
        &decoded,
        &data,
        format,
    );
    Ok(())
}

/// Read all properties of an object using RPM with PropertyIdentifier::ALL.
async fn read_all_properties<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let specs = vec![ReadAccessSpecification {
        object_identifier: oid,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::ALL,
            property_array_index: None,
        }],
    }];

    let ack = to.read_property_multiple(client, specs).await?;
    print_rpm_results(&ack.list_of_read_access_results, format);
    Ok(())
}

/// Read multiple properties from CLI string specs.
///
/// Specs format: alternating object specifiers and comma-separated property lists.
/// Example: `["ai:1", "pv,object-name", "ao:1", "pv"]`
pub async fn read_multiple_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    specs: &[String],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut access_specs = Vec::new();
    let mut i = 0;
    while i < specs.len() {
        // Try to parse as object specifier.
        let (obj_type, instance) = parse::parse_object_specifier(&specs[i])?;
        let oid = ObjectIdentifier::new(obj_type, instance)?;

        // Next arg should be comma-separated property list.
        let mut props = Vec::new();
        if i + 1 < specs.len() {
            i += 1;
            for prop_str in specs[i].split(',') {
                let (prop, idx) = parse::parse_property(prop_str.trim())?;
                props.push(PropertyReference {
                    property_identifier: prop,
                    property_array_index: idx,
                });
            }
        } else {
            // Default to PRESENT_VALUE if no properties specified.
            eprintln!(
                "Note: no properties specified for {}:{}, defaulting to present-value",
                obj_type, instance
            );
            props.push(PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            });
        }

        access_specs.push(ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: props,
        });
        i += 1;
    }

    let ack = to.read_property_multiple(client, access_specs).await?;
    print_rpm_results(&ack.list_of_read_access_results, format);
    Ok(())
}

/// Print RPM results.
fn print_rpm_results(results: &[bacnet_services::rpm::ReadAccessResult], format: OutputFormat) {
    let mut entries = Vec::new();
    for result in results {
        let obj_str = format!(
            "{}:{}",
            result.object_identifier.object_type(),
            result.object_identifier.instance_number()
        );
        let object_type = result.object_identifier.object_type();
        for elem in &result.list_of_results {
            let (value, data) = if let Some(ref value_bytes) = elem.property_value {
                let text = format_read_value(
                    object_type,
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                );
                let data = formatter(object_type).data_at(
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                );
                (text, Some(data))
            } else if let Some((class, code)) = elem.error {
                (format!("ERROR: {class}:{code}"), None)
            } else {
                ("???".to_string(), None)
            };
            entries.push(output::RpmEntry {
                object: obj_str.clone(),
                property: format!("{}", elem.property_identifier),
                index: elem.property_array_index,
                value,
                data,
            });
        }
    }
    output::print_rpm_table(&entries, format);
}

/// A read value as text: Tags as named values (the form `read` documents),
/// any other property through the shared formatter.
fn format_read_value(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    data: &[u8],
) -> String {
    if property != PropertyIdentifier::TAGS {
        return formatter(object_type).value_at(property, index, data);
    }
    match decode_tags_read(index, data) {
        Ok(TagsRead::Whole(tags)) => format!(
            "[{}]",
            tags.iter().map(format_tag).collect::<Vec<_>>().join(", ")
        ),
        Ok(TagsRead::Element(tag)) => format_tag(&tag),
        Ok(TagsRead::Size(size)) => size.to_string(),
        Err(_) => format!("[invalid Tags; raw: {}]", hex(data)),
    }
}

fn format_tag(tag: &BACnetNameValue) -> String {
    let name = serde_json::to_string(&tag.name).expect("serialize tag name");
    match &tag.value {
        None => name,
        Some(value) => {
            let value = match value {
                PropertyValue::CharacterString(text) => {
                    serde_json::to_string(text).expect("serialize tag text")
                }
                value => output::format_property_value(value),
            };
            format!("{name}={value}")
        }
    }
}

#[cfg(test)]
#[path = "read_tags_tests.rs"]
mod tags_tests;
