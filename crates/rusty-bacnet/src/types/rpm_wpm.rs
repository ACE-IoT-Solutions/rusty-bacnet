use super::*;

// ---------------------------------------------------------------------------
// RPM/WPM conversion helpers (crate-internal)
// ---------------------------------------------------------------------------

/// Decode the complete value production carried by one RPM result element.
///
/// A whole-array read can contain zero or more adjacent application values.
/// Keep ordinary and indexed one-value results scalar for API compatibility,
/// but retain the list shape for properties whose whole value is a flat list.
pub(crate) fn decode_complete_property_value(
    property: bacnet_enums::PropertyIdentifier,
    array_index: Option<u32>,
    encoded: &[u8],
) -> Result<primitives::PropertyValue, bacnet_types::error::Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < encoded.len() {
        let (value, next) = decode_application_value(encoded, offset)?;
        values.push(value);
        offset = next;
    }

    let whole_flat_list = array_index.is_none()
        && (property == bacnet_enums::PropertyIdentifier::OBJECT_LIST
            || property == bacnet_enums::PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE
            || property == bacnet_enums::PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE);

    Ok(if values.len() == 1 && !whole_flat_list {
        values.pop().expect("one decoded RPM property value")
    } else {
        primitives::PropertyValue::List(values)
    })
}

/// Convert Python RPM specs to Rust ReadAccessSpecification list.
#[allow(clippy::type_complexity)]
pub(crate) fn py_to_rpm_specs(
    specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
) -> Vec<ReadAccessSpecification> {
    specs
        .into_iter()
        .map(|(oid, props)| ReadAccessSpecification {
            object_identifier: oid.to_rust(),
            list_of_property_references: props
                .into_iter()
                .map(|(pid, idx)| PropertyReference {
                    property_identifier: pid.to_rust(),
                    property_array_index: idx,
                })
                .collect(),
        })
        .collect()
}

/// Convert a ReadPropertyMultipleACK to Python list[dict].
pub(crate) fn rpm_ack_to_py(py: Python<'_>, ack: ReadPropertyMultipleACK) -> PyResult<Py<PyAny>> {
    let outer = PyList::empty(py);
    for result in ack.list_of_read_access_results {
        let obj_dict = PyDict::new(py);
        obj_dict.set_item(
            "object_id",
            PyObjectIdentifier::from_rust(result.object_identifier),
        )?;
        let results_list = PyList::empty(py);
        for elem in result.list_of_results {
            let elem_dict = PyDict::new(py);
            elem_dict.set_item(
                "property_id",
                PyPropertyIdentifier {
                    inner: elem.property_identifier,
                },
            )?;
            elem_dict.set_item("array_index", elem.property_array_index)?;
            if let Some(value_bytes) = &elem.property_value {
                match decode_complete_property_value(
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                ) {
                    Ok(val) => {
                        elem_dict.set_item("value", PyPropertyValue::from_rust(val))?;
                    }
                    Err(_) => {
                        elem_dict.set_item("value", PyBytes::new(py, value_bytes))?;
                    }
                }
                elem_dict.set_item("error", py.None())?;
            } else if let Some((ec, ev)) = elem.error {
                elem_dict.set_item("value", py.None())?;
                let err_tuple = (PyErrorClass { inner: ec }, PyErrorCode { inner: ev });
                elem_dict.set_item("error", err_tuple)?;
            } else {
                elem_dict.set_item("value", py.None())?;
                elem_dict.set_item("error", py.None())?;
            }
            results_list.append(elem_dict)?;
        }
        obj_dict.set_item("results", results_list)?;
        outer.append(obj_dict)?;
    }
    Ok(outer.into_any().unbind())
}

/// Convert Python WPM specs to Rust WriteAccessSpecification list.
#[allow(clippy::type_complexity)]
pub(crate) fn py_to_wpm_specs(
    specs: Vec<(
        PyObjectIdentifier,
        Vec<(
            PyPropertyIdentifier,
            PyPropertyValue,
            Option<u8>,
            Option<u32>,
        )>,
    )>,
) -> Vec<WriteAccessSpecification> {
    specs
        .into_iter()
        .map(|(oid, props)| {
            let list_of_properties = props
                .into_iter()
                .map(|(pid, val, priority, array_index)| {
                    let mut buf = BytesMut::new();
                    let _ = encode_property_value(&mut buf, &val.inner);
                    BACnetPropertyValue {
                        property_identifier: pid.to_rust(),
                        property_array_index: array_index,
                        value: buf.to_vec(),
                        priority,
                    }
                })
                .collect();
            WriteAccessSpecification {
                object_identifier: oid.to_rust(),
                list_of_properties,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut bytes = BytesMut::new();
        for value in values {
            encode_property_value(&mut bytes, value).unwrap();
        }
        bytes.to_vec()
    }

    #[test]
    fn rpm_whole_object_list_preserves_zero_one_and_multiple_members() {
        let object_list = bacnet_enums::PropertyIdentifier::OBJECT_LIST;
        let first = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        );
        let second = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        );

        for members in [vec![], vec![first.clone()], vec![first, second]] {
            let decoded =
                decode_complete_property_value(object_list, None, &encoded(&members)).unwrap();
            assert_eq!(decoded, PropertyValue::List(members));
        }
    }

    #[test]
    fn rpm_scalar_and_indexed_results_remain_scalar() {
        let scalar = PropertyValue::Unsigned(7);
        assert_eq!(
            decode_complete_property_value(
                bacnet_enums::PropertyIdentifier::PRESENT_VALUE,
                None,
                &encoded(std::slice::from_ref(&scalar)),
            )
            .unwrap(),
            scalar
        );

        let member = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        );
        assert_eq!(
            decode_complete_property_value(
                bacnet_enums::PropertyIdentifier::OBJECT_LIST,
                Some(1),
                &encoded(std::slice::from_ref(&member)),
            )
            .unwrap(),
            member
        );
    }
}
