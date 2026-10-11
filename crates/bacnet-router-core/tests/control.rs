//! Strict Clause 6 network-control codec tests.

use bacnet_router_core::control::{
    decode_control, encode_control, encoded_len, ControlCodecError, InitializeRoutingTable,
    NetworkControl, RejectReason, INITIALIZE_ROUTING_TABLE, INITIALIZE_ROUTING_TABLE_ACK,
    I_AM_ROUTER_TO_NETWORK, REJECT_MESSAGE_TO_NETWORK, ROUTER_AVAILABLE_TO_NETWORK,
    ROUTER_BUSY_TO_NETWORK, WHO_IS_ROUTER_TO_NETWORK,
};

#[test]
fn who_is_router_supports_all_and_specific_queries() {
    let all = decode_control(WHO_IS_ROUTER_TO_NETWORK, &[]).unwrap();
    assert_eq!(all, NetworkControl::WhoIsRouter { network: None });

    let specific = decode_control(WHO_IS_ROUTER_TO_NETWORK, &[0x12, 0x34]).unwrap();
    let mut out = [0; 2];
    assert_eq!(encode_control(&specific, &mut out), Ok(2));
    assert_eq!(out, [0x12, 0x34]);

    for length in [1, 3, 4] {
        let payload = [0u8; 4];
        assert!(matches!(
            decode_control(WHO_IS_ROUTER_TO_NETWORK, &payload[..length]),
            Err(ControlCodecError::InvalidLength { .. })
        ));
    }
    assert!(matches!(
        decode_control(WHO_IS_ROUTER_TO_NETWORK, &[0, 0]),
        Err(ControlCodecError::InvalidNetwork(0))
    ));
}

#[test]
fn network_lists_are_exact_and_borrowed() {
    let payload = [0x00, 0x64, 0x00, 0xC8];
    let control = decode_control(I_AM_ROUTER_TO_NETWORK, &payload).unwrap();
    match control {
        NetworkControl::IAmRouter { networks } => {
            assert_eq!(networks.len(), 2);
            assert_eq!(
                networks
                    .iter()
                    .map(|network| network.get())
                    .collect::<std::vec::Vec<_>>(),
                [100, 200]
            );
            let mut out = [0xFF; 4];
            assert_eq!(encoded_len(&control), Ok(4));
            assert_eq!(encode_control(&control, &mut out), Ok(4));
            assert_eq!(out, payload);
        }
        _ => panic!("wrong control variant"),
    }

    for payload in [&[][..], &[0][..], &[0, 1, 2][..], &[0, 0, 0, 0][..]] {
        assert!(decode_control(I_AM_ROUTER_TO_NETWORK, payload).is_err());
    }
    assert!(matches!(
        decode_control(ROUTER_BUSY_TO_NETWORK, &[0, 0]),
        Err(ControlCodecError::InvalidNetwork(0))
    ));
    assert!(matches!(
        decode_control(ROUTER_AVAILABLE_TO_NETWORK, &[0xFF, 0xFF]),
        Err(ControlCodecError::InvalidNetwork(0xFFFF))
    ));
}

#[test]
fn reject_message_has_exact_length_and_reason_validation() {
    let control = decode_control(REJECT_MESSAGE_TO_NETWORK, &[4, 0x12, 0x34]).unwrap();
    assert_eq!(
        control,
        NetworkControl::RejectMessageToNetwork {
            reason: RejectReason::MessageTooLong,
            network: bacnet_router_core::NetworkNumber::new(0x1234).unwrap(),
        }
    );
    let mut out = [0; 3];
    assert_eq!(encode_control(&control, &mut out), Ok(3));
    assert_eq!(out, [4, 0x12, 0x34]);

    for payload in [&[][..], &[1, 2][..], &[1, 2, 3, 4][..]] {
        assert!(matches!(
            decode_control(REJECT_MESSAGE_TO_NETWORK, payload),
            Err(ControlCodecError::InvalidLength { .. })
        ));
    }
    assert!(matches!(
        decode_control(REJECT_MESSAGE_TO_NETWORK, &[5, 0, 1]),
        Err(ControlCodecError::InvalidRejectReason(5))
    ));
}

#[test]
fn initialize_routing_table_is_validated_before_exposure() {
    // count=2; (network, port, info-len, info)
    let payload = [2, 0, 100, 7, 2, 0xAA, 0xBB, 0, 200, 8, 0];
    let control = decode_control(INITIALIZE_ROUTING_TABLE_ACK, &payload).unwrap();
    match control {
        NetworkControl::InitializeRoutingTableAck { table } => {
            assert_eq!(table.len(), 2);
            let entries = table.iter().collect::<std::vec::Vec<_>>();
            assert_eq!(entries[0].network.get(), 100);
            assert_eq!(entries[0].port_id, 7);
            assert_eq!(entries[0].port_info, &[0xAA, 0xBB]);
            assert_eq!(entries[1].network.get(), 200);
        }
        _ => panic!("wrong control variant"),
    }

    let mut out = [0xFF; 11];
    assert_eq!(encode_control(&control, &mut out), Ok(payload.len()));
    let mut expected = [0; 11];
    expected.copy_from_slice(&payload);
    assert_eq!(out, expected);

    // Every strict prefix is rejected; no table prefix is returned.
    for length in 0..payload.len() {
        assert!(decode_control(INITIALIZE_ROUTING_TABLE_ACK, &payload[..length]).is_err());
    }
    assert!(matches!(
        decode_control(INITIALIZE_ROUTING_TABLE_ACK, &[1, 0, 100, 0, 0, 0]),
        Err(ControlCodecError::TrailingBytes { .. })
    ));
    assert!(matches!(
        decode_control(INITIALIZE_ROUTING_TABLE_ACK, &[1, 0, 0, 0, 0]),
        Err(ControlCodecError::InvalidNetwork(0))
    ));
    assert!(matches!(
        decode_control(INITIALIZE_ROUTING_TABLE_ACK, &[1, 0xFF, 0xFF, 0, 0]),
        Err(ControlCodecError::InvalidNetwork(0xFFFF))
    ));
}

#[test]
fn initialize_request_query_and_ack_zero_entry_forms_are_distinct() {
    assert_eq!(
        decode_control(INITIALIZE_ROUTING_TABLE, &[]).unwrap(),
        NetworkControl::InitializeRoutingTable {
            table: InitializeRoutingTable::Query,
        }
    );
    assert_eq!(
        decode_control(INITIALIZE_ROUTING_TABLE, &[0]).unwrap(),
        NetworkControl::InitializeRoutingTable {
            table: InitializeRoutingTable::Query,
        }
    );
    let ack = decode_control(INITIALIZE_ROUTING_TABLE_ACK, &[0]).unwrap();
    match ack {
        NetworkControl::InitializeRoutingTableAck { table } => assert_eq!(table.len(), 0),
        _ => panic!("wrong control variant"),
    }
}

#[test]
fn encoder_is_caller_buffered_and_does_not_partially_write() {
    let control = decode_control(REJECT_MESSAGE_TO_NETWORK, &[2, 0, 1]).unwrap();
    let mut out = [0xA5; 2];
    assert_eq!(
        encode_control(&control, &mut out),
        Err(ControlCodecError::BufferTooSmall {
            needed: 3,
            capacity: 2,
        })
    );
    assert_eq!(out, [0xA5, 0xA5]);
}
