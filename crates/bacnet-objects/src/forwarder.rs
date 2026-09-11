//! NotificationForwarder object (type 51) property model per ASHRAE 135-2020 Clause 12.51.
//!
//! This module models the archived registration and property surface only. It
//! does not own notification delivery or create a second notification sender;
//! forwarding execution remains the server transaction layer's responsibility.

use std::borrow::Cow;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::property_metadata::{
    property_list_from_metadata,
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};
use crate::traits::BACnetObject;

static NOTIFICATION_FORWARDER_PROPERTIES: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_NAME,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(PropertyIdentifier::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OUT_OF_SERVICE,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROCESS_IDENTIFIER_FILTER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::LOCAL_FORWARDING_ONLY,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        RequiredRead,
        None,
        ReadOnly,
    ),
];

/// BACnet NotificationForwarder object configuration/property model.
///
/// The object is inert with respect to delivery. Applications can register and
/// expose its properties without bypassing the server's notification
/// transaction ownership.
pub struct NotificationForwarderObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// Process identifier filter values exposed by the object.
    pub process_identifier_filter: Vec<u32>,
    /// Number of subscribed recipients exposed by the archived model.
    pub subscribed_recipients: u32,
    /// Whether only locally generated notifications are selected.
    pub local_forwarding_only: bool,
    /// Whether event detection is enabled.
    pub event_detection_enable: bool,
}

impl NotificationForwarderObject {
    /// Create an inert NotificationForwarder property model.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, instance)?,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            process_identifier_filter: Vec::new(),
            subscribed_recipients: 0,
            local_forwarding_only: false,
            event_detection_enable: true,
        })
    }
}

impl BACnetObject for NotificationForwarderObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            )),
            p if p == PropertyIdentifier::PROCESS_IDENTIFIER_FILTER => Ok(PropertyValue::List(
                self.process_identifier_filter
                    .iter()
                    .map(|id| PropertyValue::Unsigned(u64::from(*id)))
                    .collect(),
            )),
            p if p == PropertyIdentifier::SUBSCRIBED_RECIPIENTS => Ok(PropertyValue::Unsigned(
                u64::from(self.subscribed_recipients),
            )),
            p if p == PropertyIdentifier::LOCAL_FORWARDING_ONLY => {
                Ok(PropertyValue::Boolean(self.local_forwarding_only))
            }
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::LOCAL_FORWARDING_ONLY {
            return match value {
                PropertyValue::Boolean(value) => {
                    self.local_forwarding_only = value;
                    Ok(())
                }
                _ => Err(common::invalid_data_type_error()),
            };
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return match value {
                PropertyValue::Boolean(value) => {
                    self.event_detection_enable = value;
                    Ok(())
                }
                _ => Err(common::invalid_data_type_error()),
            };
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
        Cow::Borrowed(NOTIFICATION_FORWARDER_PROPERTIES)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        property_list_from_metadata(NOTIFICATION_FORWARDER_PROPERTIES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archived_property_model_round_trips_writable_configuration() {
        let mut object = NotificationForwarderObject::new(7, "NF-7").unwrap();
        object.process_identifier_filter = vec![100, 200];
        object.subscribed_recipients = 2;

        assert_eq!(
            object
                .read_property(PropertyIdentifier::PROCESS_IDENTIFIER_FILTER, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Unsigned(100),
                PropertyValue::Unsigned(200),
            ])
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::SUBSCRIBED_RECIPIENTS, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        for (property, value) in [
            (
                PropertyIdentifier::LOCAL_FORWARDING_ONLY,
                PropertyValue::Boolean(true),
            ),
            (
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                PropertyValue::Boolean(false),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        ] {
            object
                .write_property(property, None, value.clone(), None)
                .unwrap();
            assert_eq!(object.read_property(property, None).unwrap(), value);
        }
    }

    #[test]
    fn metadata_matches_the_actual_write_routes() {
        let object = NotificationForwarderObject::new(1, "NF").unwrap();
        for row in object.property_metadata().iter() {
            assert_eq!(
                object.is_writable_property(row.property_identifier),
                matches!(row.write_capability, Always),
                "property {}",
                row.property_identifier
            );
        }
        assert!(object
            .required_properties()
            .contains(&PropertyIdentifier::PROPERTY_LIST));
    }
}
