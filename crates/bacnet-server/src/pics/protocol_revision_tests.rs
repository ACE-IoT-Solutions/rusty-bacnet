//! The PICS takes Protocol_Version and Protocol_Revision from the Device it
//! describes, so the two can't disagree (#2125).

use std::borrow::Cow;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::{PROTOCOL_REVISION, PROTOCOL_VERSION};

use super::{PicsConfig, generate_pics};
use crate::server::test_transport::TestTransport;
use crate::server::{BACnetServer, ServerConfig};

const INSTANCE: u32 = 2125;

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(INSTANCE, 0).expect("valid Device identity")
}

fn device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: identity(),
        ..DeviceConfig::default()
    })
    .unwrap()
}

/// A library user's Device that reports its own protocol revision and
/// otherwise behaves as the bundled one.
struct RevisedDevice {
    inner: DeviceObject,
    revision: u64,
}

impl BACnetObject for RevisedDevice {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::PROTOCOL_REVISION {
            return Ok(PropertyValue::Unsigned(self.revision));
        }
        self.inner.read_property(property, array_index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner
            .write_property(property, array_index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }
}

fn database(device: Box<dyn BACnetObject>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(device).unwrap();
    db
}

fn unsigned(device: &dyn BACnetObject, property: PropertyIdentifier) -> u16 {
    match device.read_property(property, None).unwrap() {
        PropertyValue::Unsigned(raw) => u16::try_from(raw).unwrap(),
        other => panic!("{property} is {other:?}"),
    }
}

/// The bundled Device and the PICS both report the stack's constants.
#[test]
fn the_device_and_the_pics_share_the_protocol_constants() {
    let device = device();
    assert_eq!(
        unsigned(&device, PropertyIdentifier::PROTOCOL_VERSION),
        PROTOCOL_VERSION
    );
    assert_eq!(
        unsigned(&device, PropertyIdentifier::PROTOCOL_REVISION),
        PROTOCOL_REVISION
    );
    let pics = generate_pics(
        &database(Box::new(device)),
        &ServerConfig::default(),
        &PicsConfig::default(),
    );
    assert_eq!(pics.vendor_info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(pics.vendor_info.protocol_revision, PROTOCOL_REVISION);
    // The text and Markdown print the Device's values.
    let row = format!("| Protocol Revision | {PROTOCOL_REVISION} |");
    assert!(pics.generate_markdown().contains(&row));
}

/// A Device that reports another revision is what the standalone PICS states.
#[test]
fn the_standalone_pics_follows_the_devices_revision() {
    let db = database(Box::new(RevisedDevice {
        inner: device(),
        revision: 30,
    }));
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.vendor_info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(pics.vendor_info.protocol_revision, 30);
}

/// Without a Device, or with a value no `u16` holds, the constants stand in.
#[test]
fn the_constants_stand_in_without_a_usable_device_value() {
    let empty = generate_pics(
        &ObjectDatabase::new(),
        &ServerConfig::default(),
        &PicsConfig::default(),
    );
    assert_eq!(empty.vendor_info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(empty.vendor_info.protocol_revision, PROTOCOL_REVISION);
    let db = database(Box::new(RevisedDevice {
        inner: device(),
        revision: u64::from(u16::MAX) + 1,
    }));
    let wide = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(wide.vendor_info.protocol_revision, PROTOCOL_REVISION);
}

/// The server's PICS states what its served Device reports, including a
/// revision the library user's Device chose.
#[tokio::test]
async fn the_server_pics_matches_the_served_device() {
    for revision in [u64::from(PROTOCOL_REVISION), 30] {
        let server = BACnetServer::generic_builder()
            .device(identity())
            .transport(TestTransport::new())
            .database(database(Box::new(RevisedDevice {
                inner: device(),
                revision,
            })))
            .build()
            .await
            .unwrap();
        let oid = identity().device_oid();
        let mut served = [0u16; 2];
        let properties = [
            PropertyIdentifier::PROTOCOL_VERSION,
            PropertyIdentifier::PROTOCOL_REVISION,
        ];
        for (value, property) in served.iter_mut().zip(properties) {
            let PropertyValue::Unsigned(raw) =
                server.read_local(&oid, property, None).await.unwrap()
            else {
                panic!("{property} is not unsigned");
            };
            *value = u16::try_from(raw).unwrap();
        }
        let pics = server.generate_pics(&PicsConfig::default()).await;
        assert_eq!(served[1], u16::try_from(revision).unwrap());
        assert_eq!(
            [
                pics.vendor_info.protocol_version,
                pics.vendor_info.protocol_revision
            ],
            served,
            "revision {revision}"
        );
    }
}
