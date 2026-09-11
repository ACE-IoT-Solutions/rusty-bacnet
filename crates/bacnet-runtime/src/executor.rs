use std::collections::{BTreeMap, VecDeque};

use bacnet_services::rpm::ReadPropertyMultipleACK;

use crate::{ErrorCode, PropertyRead, RpmBatch, RuntimeError};

/// Stable per-property failure preserving BACnet error context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutcomeError {
    /// Machine-readable runtime category.
    pub code: ErrorCode,
    /// Whether a bounded retry may succeed without configuration changes.
    pub retryable: bool,
    /// BACnet Error Class, when returned by the device.
    pub bacnet_class: Option<u32>,
    /// BACnet Error Code, when returned by the device.
    pub bacnet_code: Option<u32>,
    /// Diagnostic detail; callers branch on `code`.
    pub message: String,
}

/// One ordered property outcome from native scan execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyOutcome {
    /// Original caller index.
    pub input_index: usize,
    /// Original property request.
    pub read: PropertyRead,
    /// Raw application-tagged BACnet value bytes.
    pub raw_value: Option<Vec<u8>>,
    /// Per-property or response-shape error.
    pub error: Option<OutcomeError>,
}

/// Pure response reconciliation used by the asynchronous scan executor.
pub struct ScanExecutor;

type ResultKey = (u32, u32, u32, Option<u32>);

impl ScanExecutor {
    /// Reconciles an RPM ACK to planned inputs without trusting response order.
    pub fn reconcile_rpm(batch: &RpmBatch, ack: ReadPropertyMultipleACK) -> Vec<PropertyOutcome> {
        let mut pending: BTreeMap<ResultKey, VecDeque<usize>> = BTreeMap::new();
        for (position, property) in batch.properties.iter().enumerate() {
            pending
                .entry(read_key(&property.read))
                .or_default()
                .push_back(position);
        }
        let mut outcomes: Vec<Option<PropertyOutcome>> = vec![None; batch.properties.len()];
        for object_result in ack.list_of_read_access_results {
            for result in object_result.list_of_results {
                let key = (
                    object_result.object_identifier.object_type().to_raw(),
                    object_result.object_identifier.instance_number(),
                    result.property_identifier.to_raw(),
                    result.property_array_index,
                );
                let Some(position) = pending.get_mut(&key).and_then(VecDeque::pop_front) else {
                    continue;
                };
                let planned = &batch.properties[position];
                let (raw_value, error) = match (result.property_value, result.error) {
                    (Some(value), None) => (Some(value), None),
                    (None, Some((class, code))) => (
                        None,
                        Some(OutcomeError {
                            code: ErrorCode::PropertyError,
                            retryable: false,
                            bacnet_class: Some(class.to_raw() as u32),
                            bacnet_code: Some(code.to_raw() as u32),
                            message: "BACnet property access error".to_owned(),
                        }),
                    ),
                    _ => (
                        None,
                        Some(decode_error(
                            "RPM result must contain exactly one of value or error",
                        )),
                    ),
                };
                outcomes[position] = Some(PropertyOutcome {
                    input_index: planned.input_index,
                    read: planned.read.clone(),
                    raw_value,
                    error,
                });
            }
        }
        outcomes
            .into_iter()
            .enumerate()
            .map(|(position, outcome)| {
                outcome.unwrap_or_else(|| {
                    let planned = &batch.properties[position];
                    PropertyOutcome {
                        input_index: planned.input_index,
                        read: planned.read.clone(),
                        raw_value: None,
                        error: Some(decode_error("RPM ACK omitted the planned property result")),
                    }
                })
            })
            .collect()
    }

    pub(crate) fn rp_success(read: &PropertyRead, raw_value: Vec<u8>) -> PropertyOutcome {
        PropertyOutcome {
            input_index: read.input_index,
            read: read.clone(),
            raw_value: Some(raw_value),
            error: None,
        }
    }

    pub(crate) fn operation_failure(read: &PropertyRead, error: &RuntimeError) -> PropertyOutcome {
        PropertyOutcome {
            input_index: read.input_index,
            read: read.clone(),
            raw_value: None,
            error: Some(OutcomeError {
                code: error.code,
                retryable: error.retryable,
                bacnet_class: None,
                bacnet_code: None,
                message: error.message.clone(),
            }),
        }
    }
}

fn read_key(read: &PropertyRead) -> ResultKey {
    (
        u32::from(read.object_type),
        read.object_instance,
        read.property_id,
        read.array_index,
    )
}

fn decode_error(message: &str) -> OutcomeError {
    OutcomeError {
        code: ErrorCode::Decode,
        retryable: false,
        bacnet_class: None,
        bacnet_code: None,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
    use bacnet_types::enums::{ErrorClass, ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    use crate::{ErrorCode, PlanLimits, PropertyRead, ScanPlanner};

    use super::ScanExecutor;

    fn read(index: usize, instance: u32, property: u32) -> PropertyRead {
        PropertyRead {
            input_index: index,
            object_type: 0,
            object_instance: instance,
            property_id: property,
            array_index: None,
            value_category: "unknown".to_owned(),
        }
    }

    #[test]
    fn reordered_partial_ack_maps_raw_values_and_errors_to_input_order() {
        let batch = ScanPlanner::plan_rpm(
            &[read(10, 1, 85), read(11, 1, 111), read(12, 2, 85)],
            PlanLimits {
                max_request_bytes: 1476,
                max_estimated_response_bytes: 10_000,
                max_properties: 10,
            },
        )
        .unwrap()
        .remove(0);
        let object = |instance| ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![
                ReadAccessResult {
                    object_identifier: object(2),
                    list_of_results: vec![ReadResultElement {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        property_value: Some(vec![0x44, 0x42, 0x91, 0x00, 0x00]),
                        error: None,
                    }],
                },
                ReadAccessResult {
                    object_identifier: object(1),
                    list_of_results: vec![
                        ReadResultElement {
                            property_identifier: PropertyIdentifier::STATUS_FLAGS,
                            property_array_index: None,
                            property_value: None,
                            error: Some((
                                ErrorClass::PROPERTY,
                                bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
                            )),
                        },
                        ReadResultElement {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            property_value: Some(vec![0x44, 0x42, 0x91, 0x00, 0x00]),
                            error: None,
                        },
                    ],
                },
            ],
        };
        let outcomes = ScanExecutor::reconcile_rpm(&batch, ack);
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| outcome.input_index)
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
        assert!(outcomes[0].raw_value.is_some());
        assert_eq!(
            outcomes[1].error.as_ref().unwrap().code,
            ErrorCode::PropertyError
        );
        assert_eq!(outcomes[1].error.as_ref().unwrap().bacnet_class, Some(2));
        assert!(outcomes[2].raw_value.is_some());
    }

    #[test]
    fn missing_result_is_explicit_decode_error_not_silent_omission() {
        let batch = ScanPlanner::plan_rpm(
            &[read(0, 1, 85)],
            PlanLimits {
                max_request_bytes: 1476,
                max_estimated_response_bytes: 1024,
                max_properties: 1,
            },
        )
        .unwrap()
        .remove(0);
        let outcomes = ScanExecutor::reconcile_rpm(
            &batch,
            ReadPropertyMultipleACK {
                list_of_read_access_results: Vec::new(),
            },
        );
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].error.as_ref().unwrap().code, ErrorCode::Decode);
    }
}
