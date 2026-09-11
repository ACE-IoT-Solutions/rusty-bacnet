use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::RuntimeError;

/// Scan scheduling class supplied by the edge property catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPriority {
    /// Identity and naming metadata.
    Identity,
    /// Operational present values.
    Value,
    /// Remaining normal-priority metadata and optional properties.
    Normal,
}

/// Validated property metadata used by the native scan planner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertySpec {
    /// Numeric BACnet property identifier.
    pub id: u32,
    /// Stable kebab-case compatibility name.
    pub name: String,
    /// Whether a device may legitimately omit the property.
    pub optional: bool,
    /// Edge value category used for decoding/projection policy.
    pub value_category: String,
    /// Native scan scheduling class.
    pub scan_priority: ScanPriority,
    /// Whether the property participates in observation planning.
    pub cov_relevant: bool,
}

/// Validated properties for one numeric BACnet object type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectCatalog {
    /// Numeric object type.
    pub object_type: u16,
    /// Stable compatibility name.
    pub name: String,
    /// Properties in the edge-authored order.
    pub properties: Vec<PropertySpec>,
}

/// Immutable, versioned property catalog loaded once per version/hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyCatalog {
    version: u32,
    sha256: [u8; 32],
    objects: BTreeMap<u16, ObjectCatalog>,
    object_names: BTreeMap<String, u16>,
    fallback: Vec<PropertySpec>,
}

#[derive(Debug, Deserialize)]
struct RawCatalog {
    catalog_version: u32,
    object_type_names: BTreeMap<String, String>,
    object_types: BTreeMap<String, RawObject>,
    property_name_ids: BTreeMap<String, u32>,
}

#[derive(Debug, Deserialize)]
struct RawObject {
    name: String,
    properties: Vec<RawProperty>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawProperty {
    id: u32,
    name: String,
    optional: bool,
    value_category: String,
    scan_priority: String,
    cov_relevant: bool,
}

impl PropertyCatalog {
    /// Parses and validates edge catalog bytes against an expected version and SHA-256.
    pub fn load(
        bytes: &[u8],
        expected_version: u32,
        expected_sha256_hex: &str,
    ) -> Result<Self, RuntimeError> {
        let actual_hash: [u8; 32] = Sha256::digest(bytes).into();
        let expected_hash = decode_sha256(expected_sha256_hex)?;
        if actual_hash != expected_hash {
            return Err(RuntimeError::invalid_config(format!(
                "property catalog SHA-256 mismatch: expected {expected_sha256_hex}, actual {}",
                encode_hex(&actual_hash)
            )));
        }
        let raw: RawCatalog = serde_json::from_slice(bytes).map_err(|error| {
            RuntimeError::invalid_config(format!("invalid property catalog JSON: {error}"))
        })?;
        if raw.catalog_version != expected_version {
            return Err(RuntimeError::invalid_config(format!(
                "property catalog version mismatch: expected {expected_version}, got {}",
                raw.catalog_version
            )));
        }

        let mut object_names = BTreeMap::new();
        for (name, raw_id) in &raw.object_type_names {
            let id = parse_object_type(raw_id)?;
            if object_names.insert(name.clone(), id).is_some() {
                return Err(RuntimeError::invalid_config(format!(
                    "duplicate object type name {name}"
                )));
            }
        }

        let mut objects = BTreeMap::new();
        for (raw_id, raw_object) in raw.object_types {
            let object_type = parse_object_type(&raw_id)?;
            if object_names.get(&raw_object.name) != Some(&object_type) {
                return Err(RuntimeError::invalid_config(format!(
                    "object type {object_type} name {} is missing or maps to a different id",
                    raw_object.name
                )));
            }
            let properties =
                validate_properties(object_type, raw_object.properties, &raw.property_name_ids)?;
            if objects
                .insert(
                    object_type,
                    ObjectCatalog {
                        object_type,
                        name: raw_object.name,
                        properties,
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::invalid_config(format!(
                    "duplicate object type id {object_type}"
                )));
            }
        }

        for (name, id) in &object_names {
            if !objects.contains_key(id) {
                return Err(RuntimeError::invalid_config(format!(
                    "object type name {name} references missing id {id}"
                )));
            }
        }

        let fallback = ["object-name", "description", "present-value"]
            .into_iter()
            .map(|name| {
                let id = raw.property_name_ids.get(name).copied().ok_or_else(|| {
                    RuntimeError::invalid_config(format!(
                        "property catalog is missing proprietary fallback {name}"
                    ))
                })?;
                Ok(PropertySpec {
                    id,
                    name: name.to_owned(),
                    optional: true,
                    value_category: "unknown".to_owned(),
                    scan_priority: if name == "present-value" {
                        ScanPriority::Value
                    } else {
                        ScanPriority::Identity
                    },
                    cov_relevant: name == "present-value",
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        Ok(Self {
            version: raw.catalog_version,
            sha256: actual_hash,
            objects,
            object_names,
            fallback,
        })
    }

    /// Catalog schema version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Lowercase SHA-256 of the exact loaded bytes.
    pub fn sha256_hex(&self) -> String {
        encode_hex(&self.sha256)
    }

    /// Looks up a standard object type by numeric identifier.
    pub fn object(&self, object_type: u16) -> Option<&ObjectCatalog> {
        self.objects.get(&object_type)
    }

    /// Number of standard object types in the loaded catalog.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Resolves a standard compatibility name to its numeric object type.
    pub fn object_type_by_name(&self, name: &str) -> Option<u16> {
        self.object_names.get(name).copied()
    }

    /// Returns standard properties or the stable proprietary fallback.
    pub fn properties_for(&self, object_type: u16) -> &[PropertySpec] {
        self.objects
            .get(&object_type)
            .map_or(&self.fallback, |object| &object.properties)
    }
}

fn validate_properties(
    object_type: u16,
    raw_properties: Vec<RawProperty>,
    property_name_ids: &BTreeMap<String, u32>,
) -> Result<Vec<PropertySpec>, RuntimeError> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    raw_properties
        .into_iter()
        .map(|property| {
            if !ids.insert(property.id) || !names.insert(property.name.clone()) {
                return Err(RuntimeError::invalid_config(format!(
                    "object type {object_type} contains duplicate property {} ({})",
                    property.name, property.id
                )));
            }
            if property_name_ids.get(&property.name) != Some(&property.id) {
                return Err(RuntimeError::invalid_config(format!(
                    "property {} ({}) disagrees with property_name_ids",
                    property.name, property.id
                )));
            }
            let scan_priority = match property.scan_priority.as_str() {
                "identity" => ScanPriority::Identity,
                "value" => ScanPriority::Value,
                "normal" => ScanPriority::Normal,
                value => {
                    return Err(RuntimeError::invalid_config(format!(
                        "property {} has unknown scan_priority {value}",
                        property.name
                    )));
                }
            };
            Ok(PropertySpec {
                id: property.id,
                name: property.name,
                optional: property.optional,
                value_category: property.value_category,
                scan_priority,
                cov_relevant: property.cov_relevant,
            })
        })
        .collect()
}

fn parse_object_type(value: &str) -> Result<u16, RuntimeError> {
    value.parse::<u16>().map_err(|error| {
        RuntimeError::invalid_config(format!("invalid object type id {value}: {error}"))
    })
}

fn decode_sha256(value: &str) -> Result<[u8; 32], RuntimeError> {
    if value.len() != 64 {
        return Err(RuntimeError::invalid_config(
            "property catalog SHA-256 must contain 64 lowercase hexadecimal characters",
        ));
    }
    let mut result = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).expect("hexadecimal input is UTF-8");
        result[index] = u8::from_str_radix(pair, 16).map_err(|_| {
            RuntimeError::invalid_config(
                "property catalog SHA-256 must contain 64 lowercase hexadecimal characters",
            )
        })?;
    }
    if encode_hex(&result) != value {
        return Err(RuntimeError::invalid_config(
            "property catalog SHA-256 must contain 64 lowercase hexadecimal characters",
        ));
    }
    Ok(result)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use crate::{ErrorCode, ScanPriority};

    use super::{encode_hex, PropertyCatalog};

    const CATALOG: &[u8] = br#"{
      "catalog_version": 1,
      "object_type_names": {"analog-input": "0"},
      "object_types": {"0": {"name": "analog-input", "properties": [
        {"id": 77, "name": "object-name", "optional": false, "value_category": "string", "scan_priority": "identity", "cov_relevant": false},
        {"id": 85, "name": "present-value", "optional": false, "value_category": "real", "scan_priority": "value", "cov_relevant": true}
      ]}},
      "property_name_ids": {"object-name": 77, "description": 28, "present-value": 85}
    }"#;

    fn hash(bytes: &[u8]) -> String {
        encode_hex(&Sha256::digest(bytes))
    }

    #[test]
    fn validates_version_hash_names_order_and_fallback() {
        let catalog = PropertyCatalog::load(CATALOG, 1, &hash(CATALOG)).unwrap();
        assert_eq!(catalog.version(), 1);
        assert_eq!(catalog.sha256_hex(), hash(CATALOG));
        assert_eq!(catalog.object_type_by_name("analog-input"), Some(0));
        assert_eq!(
            catalog
                .properties_for(0)
                .iter()
                .map(|property| (property.id, property.scan_priority))
                .collect::<Vec<_>>(),
            vec![(77, ScanPriority::Identity), (85, ScanPriority::Value)]
        );
        assert_eq!(
            catalog
                .properties_for(600)
                .iter()
                .map(|property| property.id)
                .collect::<Vec<_>>(),
            vec![77, 28, 85]
        );
    }

    #[test]
    fn rejects_hash_version_and_cross_map_drift() {
        let catalog_hash = hash(CATALOG);
        assert_eq!(
            PropertyCatalog::load(CATALOG, 2, &catalog_hash)
                .unwrap_err()
                .code,
            ErrorCode::InvalidConfig
        );
        assert_eq!(
            PropertyCatalog::load(CATALOG, 1, &"0".repeat(64))
                .unwrap_err()
                .code,
            ErrorCode::InvalidConfig
        );
        let drift = CATALOG
            .windows(b"\"object-name\": 77".len())
            .position(|window| window == b"\"object-name\": 77")
            .unwrap();
        let mut drifted = CATALOG.to_vec();
        drifted[drift + b"\"object-name\": ".len()] = b'8';
        assert_eq!(
            PropertyCatalog::load(&drifted, 1, &hash(&drifted))
                .unwrap_err()
                .code,
            ErrorCode::InvalidConfig
        );
    }
}
