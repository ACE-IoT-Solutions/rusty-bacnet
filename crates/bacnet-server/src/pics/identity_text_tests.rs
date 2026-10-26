//! The PICS takes Vendor_Name, Model_Name, Firmware_Revision and
//! Application_Software_Version from the Device it describes, and prints them
//! so that no value can break the document's structure (#2137).

use std::borrow::Cow;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{Pics, PicsConfig, PicsGenerator, generate_pics};
use crate::server::test_transport::TestTransport;
use crate::server::{BACnetServer, ServerConfig};

const INSTANCE: u32 = 2137;

/// The four strings, in the order the PICS prints them.
const TEXT: [PropertyIdentifier; 4] = [
    PropertyIdentifier::VENDOR_NAME,
    PropertyIdentifier::MODEL_NAME,
    PropertyIdentifier::FIRMWARE_REVISION,
    PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
];

fn identity(instance: u32) -> DeviceIdentity {
    DeviceIdentity::new(instance, 7)
        .expect("valid Device identity")
        .with_name(format!("Device {instance}"))
}

/// A library user's Device whose four strings the application can change:
/// a write to one stores whatever value it carries.
struct LabelledDevice {
    inner: DeviceObject,
    text: [PropertyValue; 4],
}

impl LabelledDevice {
    fn boxed(instance: u32, text: [&str; 4]) -> Box<Self> {
        let inner = DeviceObject::new(DeviceConfig {
            identity: identity(instance),
            ..DeviceConfig::default()
        })
        .unwrap();
        Box::new(Self {
            inner,
            text: text.map(|value| PropertyValue::CharacterString(value.into())),
        })
    }

    fn slot(property: PropertyIdentifier) -> Option<usize> {
        TEXT.iter().position(|p| *p == property)
    }
}

impl BACnetObject for LabelledDevice {
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
        match Self::slot(property) {
            Some(slot) => Ok(self.text[slot].clone()),
            None => self.inner.read_property(property, array_index),
        }
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        match Self::slot(property) {
            Some(slot) => {
                self.text[slot] = value;
                Ok(())
            }
            None => self
                .inner
                .write_property(property, array_index, value, priority),
        }
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

fn strings(pics: &Pics) -> [&str; 4] {
    let info = &pics.vendor_info;
    [
        &info.vendor_name,
        &info.model_name,
        &info.firmware_revision,
        &info.application_software_version,
    ]
    .map(String::as_str)
}

fn standalone(db: &ObjectDatabase) -> Pics {
    generate_pics(db, &ServerConfig::default(), &PicsConfig::default())
}

/// The bundled Device's strings are what both renderings print.
#[test]
fn the_pics_prints_the_bundled_devices_strings() {
    let device = DeviceObject::new(DeviceConfig {
        identity: identity(INSTANCE),
        vendor_name: "Example Controls".into(),
        model_name: "AHU-100".into(),
        firmware_revision: "2.1.0".into(),
        application_software_version: "2.1.1".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    let pics = standalone(&database(Box::new(device)));
    assert_eq!(
        strings(&pics),
        ["Example Controls", "AHU-100", "2.1.0", "2.1.1"]
    );
    let markdown = pics.generate_markdown();
    for row in [
        "| Vendor Name | Example Controls |",
        "| Model Name | AHU-100 |",
        "| Firmware Revision | 2.1.0 |",
        "| Application Software Version | 2.1.1 |",
    ] {
        assert!(markdown.contains(row), "{row}");
    }
    let text = pics.generate_text();
    assert!(text.contains("Model Name:                     AHU-100\n"));
}

/// A PICS generated after the application changes the Device states the new
/// values; a value that isn't a character string prints as empty.
#[test]
fn the_pics_follows_the_device_after_the_application_changes_it() {
    let mut db = database(LabelledDevice::boxed(INSTANCE, ["A", "B", "1", "2"]));
    assert_eq!(strings(&standalone(&db)), ["A", "B", "1", "2"]);
    let oid = identity(INSTANCE).device_oid();
    let device = db.get_mut(&oid).unwrap();
    for (property, value) in TEXT.into_iter().zip(["Acme", "Model 9", "3.0", "3.1"]) {
        device
            .write_property(
                property,
                None,
                PropertyValue::CharacterString(value.into()),
                None,
            )
            .unwrap();
    }
    assert_eq!(strings(&standalone(&db)), ["Acme", "Model 9", "3.0", "3.1"]);
    db.get_mut(&oid)
        .unwrap()
        .write_property(TEXT[1], None, PropertyValue::Unsigned(9), None)
        .unwrap();
    assert_eq!(strings(&standalone(&db)), ["Acme", "", "3.0", "3.1"]);
}

/// With two Devices, the server's PICS follows the one at its identity even
/// when another has a lower instance; the standalone PICS follows the
/// lowest, and so does the server's when its identity Device is missing.
#[test]
fn the_server_pics_follows_the_identity_device() {
    let mut db = database(LabelledDevice::boxed(INSTANCE, ["Served", "S", "1", "1"]));
    db.add(LabelledDevice::boxed(100, ["Lower", "L", "2", "2"]))
        .unwrap();
    let config = ServerConfig {
        device: identity(INSTANCE),
        ..ServerConfig::default()
    };
    let pics_config = PicsConfig::default();
    let server = PicsGenerator::new(&db, &config, &pics_config).for_server();
    assert_eq!(strings(&server.generate()), ["Served", "S", "1", "1"]);
    let raw = generate_pics(&db, &config, &pics_config);
    assert_eq!(strings(&raw), ["Lower", "L", "2", "2"]);
    let elsewhere = ServerConfig {
        device: identity(7),
        ..ServerConfig::default()
    };
    let server = PicsGenerator::new(&db, &elsewhere, &pics_config).for_server();
    assert_eq!(strings(&server.generate()), ["Lower", "L", "2", "2"]);
}

/// Without a Device the four strings are empty, as `PicsConfig` defaulted
/// them before.
#[test]
fn without_a_device_the_strings_are_empty() {
    let pics = standalone(&ObjectDatabase::new());
    assert_eq!(strings(&pics), ["", "", "", ""]);
    assert!(pics.generate_markdown().contains("| Vendor Name |  |\n"));
}

/// Line breaks, pipes and Markdown markup in a Device string stay inside its
/// line and cell: the document keeps its lines and the row reads as the value.
#[test]
fn device_text_cannot_break_the_document() {
    let hostile = [
        "Evil\n| Injected | row |\r\n## Heading",
        "*bold* _em_ `code` [link](x) <b>x</b> ~~s~~ &amp; \\",
        "1.0\u{7}\tbeta",
        "plain",
    ];
    let plain = standalone(&database(LabelledDevice::boxed(
        INSTANCE,
        ["a", "b", "c", "d"],
    )));
    let pics = standalone(&database(LabelledDevice::boxed(INSTANCE, hostile)));
    // VendorInfo keeps the values as read.
    assert_eq!(strings(&pics), hostile);

    let markdown = pics.generate_markdown();
    assert_eq!(
        markdown.lines().count(),
        plain.generate_markdown().lines().count()
    );
    for row in [
        r"| Vendor Name | Evil \| Injected \| row \|  ## Heading |",
        r"| Model Name | \*bold\* \_em\_ \`code\` \[link\](x) \<b\>x\</b\> \~\~s\~\~ \&amp; \\ |",
        "| Firmware Revision | 1.0  beta |",
    ] {
        assert!(
            markdown.lines().any(|line| line == row),
            "{row}\n{markdown}"
        );
    }
    assert!(!markdown.lines().any(|line| line.starts_with("## Heading")));

    let text = pics.generate_text();
    assert_eq!(text.lines().count(), plain.generate_text().lines().count());
    assert!(
        text.lines()
            .any(|line| line
                == "Vendor Name:                    Evil | Injected | row |  ## Heading"),
        "{text}"
    );
}

/// The server's PICS states what its served Device reports, before and after
/// the application changes the Device, and its vendor ID is the Device's
/// Vendor_Identifier, which the server serves from its identity.
#[tokio::test]
async fn the_server_pics_matches_the_served_device() {
    let server = BACnetServer::generic_builder()
        .device(identity(INSTANCE))
        .transport(TestTransport::new())
        .database(database(LabelledDevice::boxed(
            INSTANCE,
            ["Before", "M1", "1.0", "1.1"],
        )))
        .build()
        .await
        .unwrap();
    let oid = identity(INSTANCE).device_oid();
    for expected in [
        ["Before", "M1", "1.0", "1.1"],
        ["After", "M2", "2.0", "2.1"],
    ] {
        let pics = server.generate_pics(&PicsConfig::default()).await;
        assert_eq!(strings(&pics), expected);
        for (property, value) in TEXT.into_iter().zip(expected) {
            let served = server.read_local(&oid, property, None).await.unwrap();
            assert_eq!(served, PropertyValue::CharacterString(value.into()));
        }
        let vendor = server
            .read_local(&oid, PropertyIdentifier::VENDOR_IDENTIFIER, None)
            .await
            .unwrap();
        assert_eq!(
            vendor,
            PropertyValue::Unsigned(u64::from(pics.vendor_info.vendor_id))
        );
        let mut db = server.database().write().await;
        let device = db.get_mut(&oid).unwrap();
        for (property, value) in TEXT.into_iter().zip(["After", "M2", "2.0", "2.1"]) {
            device
                .write_property(
                    property,
                    None,
                    PropertyValue::CharacterString(value.into()),
                    None,
                )
                .unwrap();
        }
    }
}
