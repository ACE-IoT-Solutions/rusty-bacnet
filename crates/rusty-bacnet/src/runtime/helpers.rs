use super::*;

pub(super) fn runtime_error(error: RuntimeError) -> PyErr {
    let py_error = PyRuntimeError::new_err(format!("{:?}: {}", error.code, error.message));
    Python::attach(|py| {
        let value = py_error.value(py);
        let _ = value.setattr("code", format!("{:?}", error.code));
        let _ = value.setattr("retryable", error.retryable);
        if let Some(attachment_id) = error.attachment_id {
            let _ = value.setattr("attachment_id", attachment_u128(attachment_id));
        }
    });
    py_error
}

pub(super) fn attachment_u128(id: AttachmentId) -> u128 {
    u128::from_be_bytes(*id.as_bytes())
}

pub(super) fn device_key(attachment_id: u128, device_instance: u32) -> bacnet_runtime::DeviceKey {
    bacnet_runtime::DeviceKey {
        attachment_id: AttachmentId::from(attachment_id),
        device_instance,
    }
}

pub(super) fn parse_priority(value: &str) -> PyResult<WorkPriority> {
    match value {
        "foreground" => Ok(WorkPriority::Foreground),
        "poll" => Ok(WorkPriority::Poll),
        "discovery" => Ok(WorkPriority::Discovery),
        "background" => Ok(WorkPriority::Background),
        _ => Err(PyValueError::new_err(
            "priority must be foreground, poll, discovery, or background",
        )),
    }
}

pub(super) fn read_batch_request(
    reads: Vec<PyRuntimeRead>,
    timeout_ms: u64,
    priority: WorkPriority,
) -> ReadBatch {
    ReadBatch {
        items: reads
            .into_iter()
            .map(|read| ReadBatchItem {
                input_index: read.input_index,
                device: device_key(read.attachment_id, read.device_instance),
                read: PropertyRead {
                    input_index: read.input_index,
                    object_type: read.object_type,
                    object_instance: read.object_instance,
                    property_id: read.property_id,
                    array_index: read.array_index,
                    value_category: read.value_category,
                },
                freshness: read.max_age_ms.map_or(FreshnessPolicy::WireOnly, |age| {
                    FreshnessPolicy::MaxAge(Duration::from_millis(age))
                }),
            })
            .collect(),
        deadline: Instant::now() + Duration::from_millis(timeout_ms),
        priority,
    }
}

pub(super) fn write_batch_request(
    writes: Vec<PyRuntimeWrite>,
    timeout_ms: u64,
    priority: WorkPriority,
) -> WriteBatch {
    WriteBatch {
        items: writes
            .into_iter()
            .map(|write| WriteBatchItem {
                input_index: write.input_index,
                device: device_key(write.attachment_id, write.device_instance),
                target: PropertyRead {
                    input_index: write.input_index,
                    object_type: write.object_type,
                    object_instance: write.object_instance,
                    property_id: write.property_id,
                    array_index: write.array_index,
                    value_category: String::new(),
                },
                raw_value: write.raw_value,
                bacnet_priority: write.bacnet_priority,
                authorization_id: write.authorization_id,
                verify_readback: write.verify_readback,
            })
            .collect(),
        deadline: Instant::now() + Duration::from_millis(timeout_ms),
        priority,
    }
}

pub(super) fn read_outcome_to_py_with_shape(
    outcome: bacnet_runtime::ReadOutcome,
    shape: Option<(u32, Option<u32>)>,
) -> PyRuntimeBatchOutcome {
    let error_code = outcome
        .error
        .as_ref()
        .map(|error| format!("{:?}", error.code));
    let retryable = outcome.error.as_ref().is_some_and(|error| error.retryable);
    let value = match shape {
        Some((property_id, array_index)) => {
            decode_runtime_read_value(outcome.raw_value.as_deref(), property_id, array_index)
        }
        None => decode_raw_value(outcome.raw_value.as_deref()),
    };
    PyRuntimeBatchOutcome {
        input_index: outcome.input_index,
        attachment_id: attachment_u128(outcome.device.attachment_id),
        device_instance: outcome.device.device_instance,
        value,
        raw_value: outcome.raw_value,
        source: outcome
            .source
            .map(|source| format!("{source:?}").to_lowercase()),
        authorization_id: None,
        written: None,
        error_code,
        retryable,
    }
}

pub(super) fn read_value_shapes(reads: &[PyRuntimeRead]) -> HashMap<usize, (u32, Option<u32>)> {
    reads
        .iter()
        .map(|read| (read.input_index, (read.property_id, read.array_index)))
        .collect()
}

fn decode_runtime_read_value(
    raw: Option<&[u8]>,
    property_id: u32,
    array_index: Option<u32>,
) -> Option<PyPropertyValue> {
    raw.and_then(|raw| {
        crate::types::decode_complete_property_value(
            bacnet_types::enums::PropertyIdentifier::from_raw(property_id),
            array_index,
            raw,
        )
        .ok()
        .map(PyPropertyValue::from_rust)
    })
}

pub(super) fn write_outcome_to_py(outcome: bacnet_runtime::WriteOutcome) -> PyRuntimeBatchOutcome {
    let error_code = outcome
        .error
        .as_ref()
        .map(|error| format!("{:?}", error.code));
    let retryable = outcome.error.as_ref().is_some_and(|error| error.retryable);
    let value = decode_raw_value(outcome.readback.as_deref());
    PyRuntimeBatchOutcome {
        input_index: outcome.input_index,
        attachment_id: attachment_u128(outcome.device.attachment_id),
        device_instance: outcome.device.device_instance,
        value,
        raw_value: outcome.readback,
        source: None,
        authorization_id: Some(outcome.authorization_id),
        written: Some(outcome.written),
        error_code,
        retryable,
    }
}

pub(super) fn decode_raw_value(raw: Option<&[u8]>) -> Option<PyPropertyValue> {
    raw.and_then(|raw| crate::types::decode_raw_value(raw, "").ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut bytes = BytesMut::new();
        for value in values {
            encode_property_value(&mut bytes, value).unwrap();
        }
        bytes.to_vec()
    }

    #[test]
    fn runtime_whole_object_list_preserves_zero_one_and_multiple_members() {
        let first = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        );
        let second = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        );

        for members in [vec![], vec![first.clone()], vec![first, second]] {
            let decoded = decode_runtime_read_value(
                Some(&encoded(&members)),
                PropertyIdentifier::OBJECT_LIST.to_raw(),
                None,
            )
            .unwrap();
            assert_eq!(decoded.inner, PropertyValue::List(members));
        }
    }

    #[test]
    fn runtime_scalar_and_indexed_results_remain_scalar() {
        let scalar = PropertyValue::Unsigned(7);
        assert_eq!(
            decode_runtime_read_value(
                Some(&encoded(std::slice::from_ref(&scalar))),
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
                None,
            )
            .unwrap()
            .inner,
            scalar
        );

        let member = PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        );
        assert_eq!(
            decode_runtime_read_value(
                Some(&encoded(std::slice::from_ref(&member))),
                PropertyIdentifier::OBJECT_LIST.to_raw(),
                Some(1),
            )
            .unwrap()
            .inner,
            member
        );
    }
}
