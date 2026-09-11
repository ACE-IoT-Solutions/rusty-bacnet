use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleRequest};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::{PropertyCatalog, RuntimeError};

/// One ordered property read requested from the scan engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyRead {
    /// Stable caller input index.
    pub input_index: usize,
    /// Numeric object type, including proprietary values.
    pub object_type: u16,
    /// Object instance.
    pub object_instance: u32,
    /// Numeric property identifier.
    pub property_id: u32,
    /// Optional array index.
    pub array_index: Option<u32>,
    /// Catalog value category used only for response-size planning.
    pub value_category: String,
}

/// Property plus its exact position in one planned RPM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedProperty {
    /// Original caller index.
    pub input_index: usize,
    /// Specification index in the RPM.
    pub specification_index: usize,
    /// Property index within the specification.
    pub property_index: usize,
    /// Original property request.
    pub read: PropertyRead,
}

/// APDU and cardinality bounds for RPM planning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanLimits {
    /// Maximum encoded RPM service-request bytes.
    pub max_request_bytes: usize,
    /// Conservative response budget before segmentation is expected.
    pub max_estimated_response_bytes: usize,
    /// Maximum properties in one RPM regardless of byte size.
    pub max_properties: usize,
}

/// One APDU-aware RPM batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpmBatch {
    /// Encodable service request.
    pub specifications: Vec<ReadAccessSpecification>,
    /// Stable mapping back to caller inputs.
    pub properties: Vec<PlannedProperty>,
    /// Exact encoded service-request length.
    pub encoded_request_bytes: usize,
    /// Conservative response-size estimate.
    pub estimated_response_bytes: usize,
    /// True when one indivisible property is expected to require segmentation.
    pub requires_segmentation: bool,
}

/// Pure catalog expansion and APDU-aware RPM planner.
pub struct ScanPlanner;

impl ScanPlanner {
    /// Expands objects into authored catalog order without runtime reflection.
    pub fn catalog_reads(catalog: &PropertyCatalog, objects: &[(u16, u32)]) -> Vec<PropertyRead> {
        let mut reads = Vec::new();
        for (object_type, object_instance) in objects {
            for property in catalog.properties_for(*object_type) {
                reads.push(PropertyRead {
                    input_index: reads.len(),
                    object_type: *object_type,
                    object_instance: *object_instance,
                    property_id: property.id,
                    array_index: None,
                    value_category: property.value_category.clone(),
                });
            }
        }
        reads
    }

    /// Partitions ordered reads using exact request and conservative response sizes.
    pub fn plan_rpm(
        reads: &[PropertyRead],
        limits: PlanLimits,
    ) -> Result<Vec<RpmBatch>, RuntimeError> {
        if limits.max_request_bytes == 0
            || limits.max_estimated_response_bytes == 0
            || limits.max_properties == 0
        {
            return Err(RuntimeError::invalid_config(
                "RPM plan limits must all be non-zero",
            ));
        }
        let mut batches = Vec::new();
        let mut current = Vec::new();
        for read in reads {
            validate_read(read)?;
            let mut candidate = current.clone();
            candidate.push(read.clone());
            let planned = build_batch(&candidate)?;
            let exceeds = candidate.len() > limits.max_properties
                || planned.encoded_request_bytes > limits.max_request_bytes
                || planned.estimated_response_bytes > limits.max_estimated_response_bytes;
            if exceeds && !current.is_empty() {
                batches.push(build_batch(&current)?);
                current = vec![read.clone()];
            } else {
                current.push(read.clone());
            }
        }
        if !current.is_empty() {
            batches.push(build_batch(&current)?);
        }
        for batch in &mut batches {
            batch.requires_segmentation =
                batch.estimated_response_bytes > limits.max_estimated_response_bytes;
            if batch.encoded_request_bytes > limits.max_request_bytes {
                return Err(RuntimeError::invalid_config(format!(
                    "single-property RPM requires {} bytes; request budget is {}",
                    batch.encoded_request_bytes, limits.max_request_bytes
                )));
            }
        }
        Ok(batches)
    }
}

fn validate_read(read: &PropertyRead) -> Result<(), RuntimeError> {
    ObjectIdentifier::new(
        ObjectType::from_raw(u32::from(read.object_type)),
        read.object_instance,
    )
    .map(|_| ())
    .map_err(|error| RuntimeError::invalid_config(error.to_string()))
}

fn build_batch(reads: &[PropertyRead]) -> Result<RpmBatch, RuntimeError> {
    let mut specifications: Vec<ReadAccessSpecification> = Vec::new();
    let mut properties = Vec::with_capacity(reads.len());
    let mut estimated_response_bytes = 0;
    for read in reads {
        let object_identifier = ObjectIdentifier::new(
            ObjectType::from_raw(u32::from(read.object_type)),
            read.object_instance,
        )
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
        let specification_index = if specifications
            .last()
            .is_some_and(|spec| spec.object_identifier == object_identifier)
        {
            specifications.len() - 1
        } else {
            specifications.push(ReadAccessSpecification {
                object_identifier,
                list_of_property_references: Vec::new(),
            });
            specifications.len() - 1
        };
        let specification = &mut specifications[specification_index];
        let property_index = specification.list_of_property_references.len();
        specification
            .list_of_property_references
            .push(PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(read.property_id),
                property_array_index: read.array_index,
            });
        estimated_response_bytes += 12 + estimated_value_bytes(&read.value_category);
        properties.push(PlannedProperty {
            input_index: read.input_index,
            specification_index,
            property_index,
            read: read.clone(),
        });
    }
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: specifications.clone(),
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    Ok(RpmBatch {
        specifications,
        properties,
        encoded_request_bytes: encoded.len(),
        estimated_response_bytes,
        requires_segmentation: false,
    })
}

fn estimated_value_bytes(category: &str) -> usize {
    match category {
        "boolean" | "enum" | "integer" | "real" | "date" | "time" => 16,
        "bit-string" | "object-identifier" => 24,
        "string" | "octet-string" => 128,
        "collection" | "constructed" => 512,
        _ => 256,
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanLimits, PropertyRead, ScanPlanner};

    fn read(index: usize, object: u32, property: u32, category: &str) -> PropertyRead {
        PropertyRead {
            input_index: index,
            object_type: 0,
            object_instance: object,
            property_id: property,
            array_index: None,
            value_category: category.to_owned(),
        }
    }

    #[test]
    fn groups_objects_splits_on_bounds_and_retains_input_mapping() {
        let reads = vec![
            read(0, 1, 77, "string"),
            read(1, 1, 85, "real"),
            read(2, 2, 85, "real"),
            read(3, 2, 111, "bit-string"),
        ];
        let batches = ScanPlanner::plan_rpm(
            &reads,
            PlanLimits {
                max_request_bytes: 1476,
                max_estimated_response_bytes: 10_000,
                max_properties: 3,
            },
        )
        .unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].specifications.len(), 2);
        assert_eq!(
            batches
                .iter()
                .flat_map(|batch| batch.properties.iter().map(|property| property.input_index))
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert!(batches.iter().all(|batch| batch.encoded_request_bytes > 0));
    }

    #[test]
    fn indivisible_large_value_is_flagged_for_segmentation() {
        let batches = ScanPlanner::plan_rpm(
            &[read(0, 7, 600, "constructed")],
            PlanLimits {
                max_request_bytes: 64,
                max_estimated_response_bytes: 128,
                max_properties: 8,
            },
        )
        .unwrap();
        assert!(batches[0].requires_segmentation);
    }
}
