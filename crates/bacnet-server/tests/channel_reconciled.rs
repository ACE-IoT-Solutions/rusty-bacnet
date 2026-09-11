//! Public integration coverage for the restored local Channel object surface.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::lighting::ChannelObject;
use bacnet_server::pics::{generate_pics, PicsConfig};
use bacnet_server::server::ServerConfig;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

#[test]
fn channel_is_derived_into_pics_without_advertising_write_group() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ChannelObject::new(1, "Channel", 7).unwrap()))
        .unwrap();

    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    let channel = pics
        .supported_object_types
        .iter()
        .find(|support| support.object_type == ObjectType::CHANNEL)
        .expect("registered Channel must appear in the derived PICS");
    let property = |identifier| {
        channel
            .supported_properties
            .iter()
            .find(|property| property.property_id == identifier)
            .expect("implemented Channel property must appear in PICS")
    };
    assert!(property(PropertyIdentifier::PRESENT_VALUE).access.writable);
    assert!(!property(PropertyIdentifier::WRITE_STATUS).access.writable);
    assert!(!property(PropertyIdentifier::CONTROL_GROUPS).access.optional);
    assert!(pics
        .supported_services
        .iter()
        .all(|service| service.service_name != "WriteGroup"));
}
