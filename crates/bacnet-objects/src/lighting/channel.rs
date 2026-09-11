//! Channel object (type 53) per ASHRAE 135-2020 Clause 12.53.
//!
//! This object owns the local Channel property state. WriteGroup service
//! execution and remote/member propagation remain separate server concerns.

use std::borrow::Cow;

use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::property_metadata::{
    property_list_from_metadata, PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use crate::traits::BACnetObject;

const CHANNEL_PROPERTY_METADATA: &[PropertyMetadata] = &[
    row(PropertyIdentifier::OBJECT_IDENTIFIER, true, false),
    row(PropertyIdentifier::OBJECT_NAME, true, false),
    row(PropertyIdentifier::DESCRIPTION, false, true),
    row(PropertyIdentifier::OBJECT_TYPE, true, false),
    row(PropertyIdentifier::PRESENT_VALUE, true, true),
    row(PropertyIdentifier::LAST_PRIORITY, true, false),
    row(PropertyIdentifier::WRITE_STATUS, true, false),
    row(PropertyIdentifier::STATUS_FLAGS, true, false),
    row(PropertyIdentifier::RELIABILITY, true, false),
    row(PropertyIdentifier::OUT_OF_SERVICE, true, true),
    row(
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        true,
        false,
    ),
    row(PropertyIdentifier::CHANNEL_NUMBER, true, true),
    row(PropertyIdentifier::CONTROL_GROUPS, true, false),
    row(PropertyIdentifier::EXECUTION_DELAY, true, false),
    row(PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT, false, false),
    row(PropertyIdentifier::PROPERTY_LIST, true, false),
];

const fn row(
    property_identifier: PropertyIdentifier,
    required: bool,
    writable: bool,
) -> PropertyMetadata {
    PropertyMetadata::new(
        property_identifier,
        if required && writable {
            PropertyConformance::RequiredWrite
        } else if required {
            PropertyConformance::RequiredRead
        } else {
            PropertyConformance::Optional
        },
        None,
        if writable {
            PropertyWriteCapability::Always
        } else {
            PropertyWriteCapability::ReadOnly
        },
    )
}

/// BACnet Channel object with an initially empty member/control-group set.
pub struct ChannelObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    last_priority: u32,
    write_status: u32,
    channel_number: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    reliability: u32,
}

impl ChannelObject {
    /// Create a Channel with no configured member references.
    pub fn new(instance: u32, name: impl Into<String>, channel_number: u32) -> Result<Self, Error> {
        if channel_number == 0 {
            return Err(common::value_out_of_range_error());
        }
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::CHANNEL, instance)?,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            last_priority: 16,
            write_status: 0,
            channel_number,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
        })
    }

    /// Set the optional Description property.
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    fn read_empty_array(array_index: Option<u32>) -> Result<PropertyValue, Error> {
        match array_index {
            None => Ok(PropertyValue::List(Vec::new())),
            Some(0) => Ok(PropertyValue::Unsigned(0)),
            Some(_) => Err(common::invalid_array_index_error()),
        }
    }
}

impl BACnetObject for ChannelObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::CHANNEL.to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            PropertyIdentifier::LAST_PRIORITY => {
                Ok(PropertyValue::Unsigned(self.last_priority as u64))
            }
            PropertyIdentifier::WRITE_STATUS => Ok(PropertyValue::Enumerated(self.write_status)),
            PropertyIdentifier::CHANNEL_NUMBER => {
                Ok(PropertyValue::Unsigned(self.channel_number as u64))
            }
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
            | PropertyIdentifier::CONTROL_GROUPS
            | PropertyIdentifier::EXECUTION_DELAY => Self::read_empty_array(array_index),
            PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT => Ok(PropertyValue::Boolean(false)),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            let PropertyValue::Unsigned(value) = value else {
                return Err(common::invalid_data_type_error());
            };
            self.present_value = common::u64_to_u32(value)?;
            self.last_priority = priority.unwrap_or(16) as u32;
            self.write_status = 2; // successful: the configured member set is empty
            return Ok(());
        }
        if property == PropertyIdentifier::CHANNEL_NUMBER {
            let PropertyValue::Unsigned(value) = value else {
                return Err(common::invalid_data_type_error());
            };
            let value = common::u64_to_u32(value)?;
            if value == 0 {
                return Err(common::value_out_of_range_error());
            }
            self.channel_number = value;
            return Ok(());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(CHANNEL_PROPERTY_METADATA)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        property_list_from_metadata(CHANNEL_PROPERTY_METADATA)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::PRESENT_VALUE
                | PropertyIdentifier::LAST_PRIORITY
                | PropertyIdentifier::WRITE_STATUS
                | PropertyIdentifier::STATUS_FLAGS
        )
    }

    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::PROPERTY_LIST
                | PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
                | PropertyIdentifier::CONTROL_GROUPS
                | PropertyIdentifier::EXECUTION_DELAY
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_current_array_and_metadata_shapes() {
        let channel = ChannelObject::new(1, "Channel", 7).unwrap();
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
                .unwrap(),
            PropertyValue::Unsigned(7)
        );
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
                .unwrap(),
            PropertyValue::List(Vec::new())
        );
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::CONTROL_GROUPS, Some(0))
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert!(channel.is_array_property(PropertyIdentifier::EXECUTION_DELAY));
        assert!(channel.supports_cov());
        assert!(channel.supports_cov_property(PropertyIdentifier::WRITE_STATUS));
    }

    #[test]
    fn present_value_updates_exact_cov_coordinates() {
        let mut channel = ChannelObject::new(1, "Channel", 7).unwrap();
        channel
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(42),
                Some(8),
            )
            .unwrap();
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(42)
        );
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::LAST_PRIORITY, None)
                .unwrap(),
            PropertyValue::Unsigned(8)
        );
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::WRITE_STATUS, None)
                .unwrap(),
            PropertyValue::Enumerated(2)
        );
    }

    #[test]
    fn channel_number_is_nonzero_and_writes_are_failure_atomic() {
        assert!(ChannelObject::new(1, "Channel", 0).is_err());
        let mut channel = ChannelObject::new(1, "Channel", 7).unwrap();
        assert!(channel
            .write_property(
                PropertyIdentifier::CHANNEL_NUMBER,
                None,
                PropertyValue::Unsigned(0),
                None,
            )
            .is_err());
        assert_eq!(
            channel
                .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
                .unwrap(),
            PropertyValue::Unsigned(7)
        );
    }
}
