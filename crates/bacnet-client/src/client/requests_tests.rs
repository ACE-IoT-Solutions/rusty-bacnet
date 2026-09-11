use super::{check_transmittable_length, LengthBoundedBy};
use crate::client::BACnetClient;
use bacnet_encoding::{apdu, npdu::decode_npdu};
use bacnet_transport::{loopback::LoopbackTransport, port::TransportPort};
use bacnet_types::enums::UnconfirmedServiceChoice;
use tokio::time::{timeout, Duration};

#[test]
fn conformant_lengths_pass() {
    for advertised in [50u16, 128, 206, 480, 1024, 1476] {
        assert!(
            check_transmittable_length(LengthBoundedBy::DiscoveredPeer(advertised), 1476).is_ok(),
            "{advertised} is at or above MinimumMessageSize"
        );
    }
}

/// I-Am carries an Unsigned octet count, not the four-bit code, and Clause
/// 20.1.2.5 says the true value "may be larger than indicated in this
/// parameter" — so values outside the six encodings are legitimate.
#[test]
fn lengths_outside_the_encoded_set_are_not_rejected() {
    for advertised in [51u16, 600, 1500, u16::MAX] {
        assert!(
            check_transmittable_length(LengthBoundedBy::DiscoveredPeer(advertised), u16::MAX)
                .is_ok(),
            "{advertised} is conformant even though it is not one of the six encodings"
        );
    }
}

/// The error must blame whichever term actually bound the minimum. The peer
/// is only sometimes that term: BACnet/SC recomputes its own limit from the
/// hub's Connect-Accept, so a transport can fall below the floor while the
/// peer is entirely conformant.
#[test]
fn the_error_names_the_binding_term() {
    let peer_bound =
        check_transmittable_length(LengthBoundedBy::DiscoveredPeer(3), 1476).unwrap_err();
    assert!(
        peer_bound.to_string().contains("peer's advertised"),
        "peer is the binding term here, got: {peer_bound}"
    );

    let transport_bound =
        check_transmittable_length(LengthBoundedBy::DiscoveredPeer(1476), 48).unwrap_err();
    assert!(
        transport_bound.to_string().contains("transport's limit"),
        "transport is the binding term here and the peer is conformant, got: {transport_bound}"
    );
    assert!(
        !transport_bound.to_string().contains("peer's advertised"),
        "must not blame a conformant peer for the transport's limit"
    );
}

#[tokio::test]
async fn routed_unconfirmed_preserves_explicit_network_and_multibyte_address() {
    let client_mac = vec![0x11];
    let router_mac = vec![0x22];
    let remote_network = 0x3456;
    let remote_mac = vec![0xaa, 0xbb, 0xcc, 0xdd];
    let service_data = [0x1a, 0x2b, 0x3c];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    client
        .unconfirmed_request_routed(
            &router_mac,
            remote_network,
            &remote_mac,
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &service_data,
        )
        .await
        .unwrap();

    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for routed unconfirmed request")
        .expect("router channel closed");
    assert_eq!(&received.source_mac[..], &client_mac);
    let npdu = decode_npdu(received.npdu).unwrap();
    let destination = npdu.destination.expect("routed destination");
    assert_eq!(destination.network, remote_network);
    assert_eq!(&destination.mac_address[..], &remote_mac);
    let apdu::Apdu::UnconfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected an unconfirmed request")
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
    );
    assert_eq!(&request.service_request[..], &service_data);

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}
