use super::*;

#[test]
fn generate_pics_basic() {
    let db = make_test_db();
    let server_config = ServerConfig {
        vendor_id: 999,
        runtime_capabilities: RuntimeCapabilities::bip_v4(),
        ..ServerConfig::default()
    };
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    assert_eq!(pics.vendor_info.vendor_id, 999);
    assert_eq!(pics.vendor_info.vendor_name, "Acme Corp");
    assert_eq!(pics.device_profile, DeviceProfile::BAsc);
    assert_eq!(pics.character_sets, vec![CharacterSet::Utf8]);
    assert_eq!(pics.data_link_layers, vec![DataLinkSupport::BipV4]);
}

#[test]
fn vendor_info_prefers_readable_live_device_properties() {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};

    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 42,
            name: "Live Device".into(),
            vendor_name: "Live Vendor".into(),
            vendor_id: 321,
            model_name: "Live Model".into(),
            firmware_revision: "fw-live".into(),
            application_software_version: "app-live".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let config = ServerConfig {
        vendor_id: 999,
        ..ServerConfig::default()
    };
    let pics = generate_pics(&db, &config, &make_pics_config());

    assert_eq!(pics.vendor_info.vendor_id, 321);
    assert_eq!(pics.vendor_info.vendor_name, "Live Vendor");
    assert_eq!(pics.vendor_info.model_name, "Live Model");
    assert_eq!(pics.vendor_info.firmware_revision, "fw-live");
    assert_eq!(pics.vendor_info.application_software_version, "app-live");
    assert_eq!(pics.vendor_info.protocol_version, 1);
    assert_eq!(pics.vendor_info.protocol_revision, 22);
}

#[test]
fn runtime_capabilities_are_authoritative_over_requested_pics_claims() {
    let db = make_test_db();
    let requested = make_pics_config();

    let generic = generate_pics(&db, &ServerConfig::default(), &requested);
    assert!(generic.data_link_layers.is_empty());
    assert_eq!(generic.network_layer, NetworkLayerSupport::default());

    let capabilities = RuntimeCapabilities {
        data_link_layers: vec![DataLinkSupport::BacnetSc],
        network_layer: NetworkLayerSupport {
            router: true,
            bbmd: false,
            foreign_device: false,
        },
    };
    let configured = ServerConfig {
        runtime_capabilities: capabilities.clone(),
        ..ServerConfig::default()
    };
    let pics = generate_pics(&db, &configured, &requested);
    assert_eq!(pics.data_link_layers, capabilities.data_link_layers);
    assert_eq!(pics.network_layer, capabilities.network_layer);

    let mut router_requested = requested;
    router_requested.device_profile = DeviceProfile::BRouter;
    let generic = generate_pics(&db, &ServerConfig::default(), &router_requested);
    assert_ne!(generic.device_profile, DeviceProfile::BRouter);
    let routed = generate_pics(&db, &configured, &router_requested);
    assert_eq!(routed.device_profile, DeviceProfile::BRouter);
}

#[test]
fn all_object_types_listed() {
    let db = make_test_db();
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    let types: Vec<ObjectType> = pics
        .supported_object_types
        .iter()
        .map(|ot| ot.object_type)
        .collect();
    assert!(types.contains(&ObjectType::DEVICE));
    assert!(types.contains(&ObjectType::ANALOG_INPUT));
    assert!(types.contains(&ObjectType::BINARY_VALUE));
    // 3 distinct types in our test DB
    assert_eq!(types.len(), 3);
}

#[test]
fn object_type_properties_populated() {
    let db = make_test_db();
    let unmigrated = db
        .iter_objects()
        .find_map(|(_, object)| {
            (object.object_identifier().object_type() == ObjectType::ANALOG_INPUT).then_some(object)
        })
        .unwrap();
    assert!(
        unmigrated.property_metadata().is_empty(),
        "the test fixture must exercise the legacy PICS fallback"
    );
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    let ai = pics
        .supported_object_types
        .iter()
        .find(|ot| ot.object_type == ObjectType::ANALOG_INPUT)
        .expect("ANALOG_INPUT should be in PICS");

    // AI has 8 properties in our test fixture
    assert_eq!(ai.supported_properties.len(), 8);

    // The TestAnalogInput stub does not override is_writable_property, so it
    // inherits the default historical_writable_default heuristic, which reports
    // PRESENT_VALUE read-only for ANALOG_INPUT. The real AnalogInputObject
    // overrides this to writable-when-out-of-service — see
    // pics_input_present_value_writable_only_when_out_of_service.
    let pv = ai
        .supported_properties
        .iter()
        .find(|p| p.property_id == PropertyIdentifier::PRESENT_VALUE)
        .expect("PRESENT_VALUE should exist");
    assert!(pv.access.readable);
    assert!(!pv.access.writable);
}

#[test]
fn device_not_createable_or_deleteable() {
    let db = make_test_db();
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    let dev = pics
        .supported_object_types
        .iter()
        .find(|ot| ot.object_type == ObjectType::DEVICE)
        .expect("DEVICE should be in PICS");
    assert!(!dev.createable);
    assert!(!dev.deleteable);
}

#[test]
fn real_device_and_network_port_overrides_not_createable_or_deleteable() {
    // Directly exercise the real DeviceObject and NetworkPortObject trait
    // overrides (not the TestDevice stub) so a regression flipping either
    // override to true fails here rather than silently changing PICS output.
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::network_port::NetworkPortObject;

    let device = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "Dev".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(!device.is_createable(), "Device must not be createable");
    assert!(!device.is_deleteable(), "Device must not be deleteable");

    let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
    assert!(!np.is_createable(), "NetworkPort must not be createable");
    assert!(!np.is_deleteable(), "NetworkPort must not be deleteable");
}

#[test]
fn services_match_implementation() {
    let db = make_test_db();
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    let service_names: Vec<&str> = pics
        .supported_services
        .iter()
        .map(|s| s.service_name.as_str())
        .collect();

    // Executor services
    assert!(service_names.contains(&"ReadProperty"));
    assert!(service_names.contains(&"WriteProperty"));
    assert!(service_names.contains(&"ReadPropertyMultiple"));
    assert!(service_names.contains(&"SubscribeCOV"));
    assert!(service_names.contains(&"CreateObject"));
    assert!(service_names.contains(&"DeleteObject"));
    assert!(service_names.contains(&"WhoIs"));
    assert!(
        !service_names.contains(&"WriteGroup"),
        "server PICS must not list unsupported inbound WriteGroup"
    );

    // Initiator services
    assert!(service_names.contains(&"I-Am"));
    assert!(service_names.contains(&"ConfirmedCOVNotification"));

    // Check initiator/executor flags on ReadProperty
    let rp = pics
        .supported_services
        .iter()
        .find(|s| s.service_name == "ReadProperty")
        .expect("ReadProperty should be listed");
    assert!(!rp.initiator);
    assert!(rp.executor);

    // I-Am is initiator only
    let iam = pics
        .supported_services
        .iter()
        .find(|s| s.service_name == "I-Am")
        .expect("I-Am should be listed");
    assert!(iam.initiator);
    assert!(!iam.executor);
}

#[test]
fn text_output_contains_key_sections() {
    let db = make_test_db();
    let server_config = ServerConfig {
        vendor_id: 42,
        runtime_capabilities: RuntimeCapabilities::bip_v4(),
        ..ServerConfig::default()
    };
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);
    let text = pics.generate_text();

    assert!(text.contains("Protocol Implementation Conformance Statement"));
    assert!(text.contains("Vendor ID:"));
    assert!(text.contains("42"));
    assert!(text.contains("Acme Corp"));
    assert!(text.contains("B-ASC"));
    assert!(text.contains("Supported Object Types"));
    assert!(text.contains("ANALOG_INPUT"));
    assert!(text.contains("Supported Services"));
    assert!(text.contains("ReadProperty"));
    assert!(text.contains("Data Link Layer Support"));
    assert!(text.contains("BACnet/IP (Annex J)"));
    assert!(text.contains("Character Sets Supported"));
    assert!(text.contains("UTF-8"));
    assert!(text.contains("Special Functionality"));
    assert!(text.contains("Intrinsic event reporting"));
}

#[test]
fn markdown_output_has_tables() {
    let db = make_test_db();
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);
    let md = pics.generate_markdown();

    assert!(md.contains("# BACnet Protocol Implementation Conformance Statement"));
    assert!(md.contains("| Field | Value |"));
    assert!(md.contains("| Service | Initiator | Executor |"));
    assert!(md.contains("| Property | Access |"));
    assert!(md.contains("## Supported Object Types"));
    assert!(md.contains("## Supported Services"));
}

#[test]
fn empty_database_produces_empty_object_list() {
    let db = ObjectDatabase::new();
    let server_config = ServerConfig::default();
    let pics_config = PicsConfig::default();
    let pics = generate_pics(&db, &server_config, &pics_config);

    assert!(pics.supported_object_types.is_empty());
    assert!(!pics.supported_services.is_empty());
}

#[test]
fn device_profile_display() {
    assert_eq!(DeviceProfile::BAac.to_string(), "B-AAC");
    assert_eq!(DeviceProfile::BAsc.to_string(), "B-ASC");
    assert_eq!(DeviceProfile::BOws.to_string(), "B-OWS");
    assert_eq!(DeviceProfile::BBc.to_string(), "B-BC");
    assert_eq!(DeviceProfile::BOp.to_string(), "B-OP");
    assert_eq!(DeviceProfile::BRouter.to_string(), "B-ROUTER");
    assert_eq!(DeviceProfile::BGw.to_string(), "B-GW");
    assert_eq!(DeviceProfile::BSc.to_string(), "B-SC");
    assert_eq!(
        DeviceProfile::Custom("MyProfile".into()).to_string(),
        "MyProfile"
    );
}

#[test]
fn property_access_display() {
    let rw = PropertyAccess {
        readable: true,
        writable: true,
        optional: false,
    };
    assert_eq!(rw.to_string(), "RW");

    let ro = PropertyAccess {
        readable: true,
        writable: false,
        optional: true,
    };
    assert_eq!(ro.to_string(), "RO");

    let wo = PropertyAccess {
        readable: false,
        writable: true,
        optional: false,
    };
    assert_eq!(wo.to_string(), "W");
}

#[test]
fn binary_value_present_value_is_writable() {
    let db = make_test_db();
    let server_config = ServerConfig::default();
    let pics_config = make_pics_config();
    let pics = generate_pics(&db, &server_config, &pics_config);

    let bv = pics
        .supported_object_types
        .iter()
        .find(|ot| ot.object_type == ObjectType::BINARY_VALUE)
        .expect("BINARY_VALUE should be in PICS");

    let pv = bv
        .supported_properties
        .iter()
        .find(|p| p.property_id == PropertyIdentifier::PRESENT_VALUE)
        .expect("PRESENT_VALUE should exist on BV");
    assert!(
        pv.access.writable,
        "BinaryValue PRESENT_VALUE should be writable"
    );
}
