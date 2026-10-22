//! Malformed and unsupported inputs never become discovery observations.
use super::*;
fn wire(message: Bvll<'_>) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    let len = message.encode_into(&mut bytes).unwrap();
    bytes.truncate(len);
    bytes
}
fn peer() -> SocketAddrV4 {
    "127.0.0.1:47808".parse().unwrap()
}
#[test]
fn passive_native_router_reports_validate_payload_and_delivery_kind() {
    let router = [1, 0x80, 1, 0, 2, 0, 3];
    assert_eq!(
        decode(&wire(Bvll::OriginalBroadcastNpdu(&router)), peer()),
        Some(DiscoveryEvent::Router {
            router: peer().to_string(),
            networks: vec![2, 3]
        })
    );
    // A routed announcement's station and immediate next hop remain distinct.
    let routed = [1, 0x88, 0, 7, 1, 5, 1, 0, 2];
    assert_eq!(
        decode(&wire(Bvll::OriginalUnicastNpdu(&routed)), peer()),
        Some(DiscoveryEvent::Router {
            router: "7:0x05 via 127.0.0.1:47808".into(),
            networks: vec![2]
        })
    );
    let number = [1, 0x80, 0x13, 0, 77, 1];
    assert_eq!(
        decode(&wire(Bvll::OriginalBroadcastNpdu(&number)), peer()),
        Some(DiscoveryEvent::NetworkNumber {
            network: 77,
            configured: true
        })
    );
    assert_eq!(
        decode(&wire(Bvll::OriginalUnicastNpdu(&number)), peer()),
        None
    );
    for payload in [
        &[1, 0x80, 1][..],
        &[1, 0x80, 1, 0],
        &[1, 0x80, 1, 0, 0],
        &[1, 0x80, 1, 255, 255],
        &[1, 0x80, 0x13, 0, 77, 2],
        &[1, 0x88, 0, 7, 1, 5, 0x13, 0, 77, 1],
        &[1, 0x80, 0],
        &[1, 0, 0x10, 1],
        &[2, 0],
        &[1],
    ] {
        assert_eq!(
            decode(&wire(Bvll::OriginalBroadcastNpdu(payload)), peer()),
            None,
            "{payload:?}"
        );
    }
}
#[test]
fn passive_rejects_truncation_wrong_bvll_and_management_wrapped_data() {
    let router = [1, 0x80, 1, 0, 2];
    let valid = wire(Bvll::ForwardedNpdu {
        origin: peer(),
        npdu: &router,
    });
    for length in 0..valid.len() {
        assert_eq!(decode(&valid[..length], peer()), None);
    }
    for (offset, value) in [(0, 0x82), (1, 0xFF), (2, 0xFF), (3, 0)] {
        let mut changed = valid.clone();
        changed[offset] = value;
        assert_eq!(decode(&changed, peer()), None);
    }
    assert_eq!(
        decode(&wire(Bvll::DistributeBroadcastToNetwork(&router)), peer()),
        None
    );
    assert_eq!(
        decode(&wire(Bvll::ReadBroadcastDistributionTable), peer()),
        None
    );
}
#[test]
fn passive_native_ihave_requires_complete_service_and_device_identifier() {
    use bacnet_encoding::{
        apdu::{UnconfirmedRequest, encode_apdu},
        npdu::{Npdu, encode_npdu},
    };
    use bacnet_types::primitives::ObjectIdentifier;
    use bytes::BytesMut;
    let mut service = BytesMut::new();
    IHaveRequest {
        device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 12).unwrap(),
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        object_name: "Supply".into(),
    }
    .encode(&mut service)
    .unwrap();
    let encoded = |body: &[u8]| {
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: UnconfirmedServiceChoice::I_HAVE,
                service_request: Bytes::copy_from_slice(body),
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: apdu.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        wire(Bvll::OriginalBroadcastNpdu(&npdu))
    };
    assert!(
        matches!(decode(&encoded(&service), peer()), Some(DiscoveryEvent::IHave(row)) if row.name == "Supply" && row.device == 12)
    );
    for length in 0..service.len() {
        assert_eq!(decode(&encoded(&service[..length]), peer()), None);
    }
    service.extend_from_slice(&[0]);
    assert_eq!(decode(&encoded(&service), peer()), None, "trailing member");
    service.truncate(service.len() - 1);
    service[1] = 0; // Device object type becomes Analog Input.
    assert_eq!(decode(&encoded(&service), peer()), None);
}
