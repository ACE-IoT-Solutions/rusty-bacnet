use std::borrow::Cow;

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::*;

// ── Minimal test objects ───────────────────────────────────────────

struct TestAnalogInput {
    oid: ObjectIdentifier,
    name: String,
}

impl BACnetObject for TestAnalogInput {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Real(0.0))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: [PropertyIdentifier; 8] = [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::UNITS,
        ];
        Cow::Borrowed(&PROPS)
    }
}

struct TestBinaryValue {
    oid: ObjectIdentifier,
    name: String,
}

impl BACnetObject for TestBinaryValue {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Boolean(false))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: [PropertyIdentifier; 6] = [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ];
        Cow::Borrowed(&PROPS)
    }
}

struct TestDevice {
    oid: ObjectIdentifier,
    name: String,
}

impl BACnetObject for TestDevice {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Unsigned(0))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: [PropertyIdentifier; 6] = [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::PROTOCOL_VERSION,
            PropertyIdentifier::PROTOCOL_REVISION,
        ];
        Cow::Borrowed(&PROPS)
    }
    /// Device is not createable or deleteable; mirrors the real DeviceObject.
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn make_test_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(TestDevice {
        oid: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        name: "Test Device".into(),
    }))
    .unwrap();
    db.add(Box::new(TestAnalogInput {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        name: "AI-1".into(),
    }))
    .unwrap();
    db.add(Box::new(TestAnalogInput {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
        name: "AI-2".into(),
    }))
    .unwrap();
    db.add(Box::new(TestBinaryValue {
        oid: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap(),
        name: "BV-1".into(),
    }))
    .unwrap();
    db
}

fn make_pics_config() -> PicsConfig {
    PicsConfig {
        vendor_name: "Acme Corp".into(),
        model_name: "BACnet Controller 3000".into(),
        firmware_revision: "1.0.0".into(),
        application_software_version: "2.0.0".into(),
        protocol_version: 1,
        protocol_revision: 24,
        device_profile: DeviceProfile::BAsc,
        data_link_layers: vec![DataLinkSupport::BipV4],
        network_layer: NetworkLayerSupport {
            router: false,
            bbmd: false,
            foreign_device: false,
        },
        character_sets: vec![CharacterSet::Utf8],
        special_functionality: vec!["Intrinsic event reporting".into()],
    }
}

mod basic;
mod runtime_contract;
