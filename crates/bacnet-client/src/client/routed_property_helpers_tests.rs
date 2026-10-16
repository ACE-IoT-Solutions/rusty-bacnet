use super::*;
use bacnet_encoding::apdu::{self, Apdu, ComplexAck};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

#[tokio::test]
async fn explicit_routed_rpm_preserves_dadr_and_decodes_ack() {
    let (transport, mut router) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut incoming = router.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    let object_identifier = ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap();
    let specs = vec![ReadAccessSpecification {
        object_identifier,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::OBJECT_NAME,
            property_array_index: None,
        }],
    }];
    let send = client.read_property_multiple_routed(&[2], 200, &[0, 42], specs);
    let reply = async {
        let received = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(received.npdu).unwrap();
        let destination = npdu.destination.unwrap();
        assert_eq!(destination.network, 200);
        assert_eq!(destination.mac_address.as_slice(), &[0, 42]);
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected routed RPM request")
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
        );
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier,
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    property_value: Some(vec![0x21, 42]),
                    error: None,
                }],
            }],
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        super::tests::send_routed_response(
            &router,
            &[1],
            200,
            &[0, 42],
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                service_ack: service_ack.freeze(),
            }),
        )
        .await;
    };
    let (result, ()) = tokio::join!(send, reply);
    let ack = result.unwrap();
    assert_eq!(
        ack.list_of_read_access_results[0].list_of_results[0]
            .property_value
            .as_deref(),
        Some(&[0x21, 42][..])
    );
    client.stop().await.unwrap();
    router.stop().await.unwrap();
}
