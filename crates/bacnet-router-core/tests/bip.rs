//! BACnet/IP codec and UDP provenance tests.

use bacnet_router_core::{
    decode_bip_datagram, encode_bvll, encode_forwarded_npdu, BipDecodeError, BvlcFunction,
    Ipv4SocketAddress, UdpDatagramMeta,
};

fn meta(group: bool) -> UdpDatagramMeta {
    UdpDatagramMeta {
        peer: Ipv4SocketAddress {
            ip: [192, 0, 2, 10],
            port: 47_809,
        },
        destination: Some(if group {
            [192, 0, 2, 255]
        } else {
            [192, 0, 2, 20]
        }),
        destination_is_group: group,
        truncated: false,
    }
}

#[test]
fn original_functions_require_matching_udp_destination_provenance() {
    let unicast = [0x81, 0x0a, 0x00, 0x06, 0x01, 0x00];
    let broadcast = [0x81, 0x0b, 0x00, 0x06, 0x01, 0x00];

    let decoded = decode_bip_datagram(&unicast, meta(false)).unwrap();
    assert_eq!(decoded.function, BvlcFunction::OriginalUnicastNpdu);
    assert_eq!(decoded.logical_source, meta(false).peer);
    assert_eq!(decoded.npdu, [0x01, 0x00]);
    assert!(!decoded.link_layer_group);

    assert_eq!(
        decode_bip_datagram(&unicast, meta(true)),
        Err(BipDecodeError::DestinationClassMismatch)
    );
    assert_eq!(
        decode_bip_datagram(&broadcast, meta(false)),
        Err(BipDecodeError::DestinationClassMismatch)
    );
    assert!(decode_bip_datagram(&broadcast, meta(true)).is_ok());
}

#[test]
fn forwarded_npdu_keeps_origin_and_immediate_peer_distinct() {
    let frame = [
        0x81, 0x04, 0x00, 0x0c, 198, 51, 100, 7, 0xba, 0xc1, 0x01, 0x00,
    ];
    let decoded = decode_bip_datagram(&frame, meta(false)).unwrap();
    assert_eq!(decoded.function, BvlcFunction::ForwardedNpdu);
    assert_eq!(decoded.immediate_peer, meta(false).peer);
    assert_eq!(
        decoded.logical_source,
        Ipv4SocketAddress {
            ip: [198, 51, 100, 7],
            port: 0xbac1,
        }
    );
    assert_eq!(decoded.npdu, [0x01, 0x00]);
}

#[test]
fn datagram_length_is_exact_and_destination_metadata_is_required() {
    let trailing = [0x81, 0x0a, 0x00, 0x06, 0x01, 0x00, 0xde, 0xad];
    assert_eq!(
        decode_bip_datagram(&trailing, meta(false)),
        Err(BipDecodeError::LengthMismatch {
            declared: 6,
            actual: 8,
        })
    );

    let mut missing = meta(false);
    missing.destination = None;
    assert_eq!(
        decode_bip_datagram(&trailing[..6], missing),
        Err(BipDecodeError::MissingDestinationMetadata)
    );
}

#[test]
fn caller_owned_encoders_produce_borrowed_decoder_vectors() {
    let npdu = [0x01, 0x20, 0xff, 0xff, 0x00, 0xff, 0x10];
    let mut output = [0u8; 32];
    let len = encode_bvll(&mut output, BvlcFunction::OriginalBroadcastNpdu, &npdu).unwrap();
    assert_eq!(&output[..4], &[0x81, 0x0b, 0x00, len as u8]);
    assert_eq!(
        decode_bip_datagram(&output[..len], meta(true))
            .unwrap()
            .npdu,
        npdu
    );

    let origin = Ipv4SocketAddress {
        ip: [203, 0, 113, 8],
        port: 47_841,
    };
    let len = encode_forwarded_npdu(&mut output, origin, &npdu).unwrap();
    let decoded = decode_bip_datagram(&output[..len], meta(false)).unwrap();
    assert_eq!(decoded.logical_source, origin);
    assert_eq!(decoded.npdu, npdu);
}

#[test]
fn malformed_or_unsupported_datagrams_fail_closed() {
    let mut truncated_meta = meta(false);
    truncated_meta.truncated = true;
    assert_eq!(
        decode_bip_datagram(&[0x81, 0x0a, 0x00, 0x04], truncated_meta),
        Err(BipDecodeError::DatagramTruncated)
    );
    assert_eq!(
        decode_bip_datagram(&[0x82, 0x0a, 0x00, 0x04], meta(false)),
        Err(BipDecodeError::WrongType(0x82))
    );
    assert_eq!(
        decode_bip_datagram(&[0x81, 0x00, 0x00, 0x04], meta(false)),
        Err(BipDecodeError::UnsupportedFunction(0x00))
    );
    assert_eq!(
        decode_bip_datagram(&[0x81, 0x04, 0x00, 0x09, 1, 2, 3, 4, 5], meta(false)),
        Err(BipDecodeError::ForwardedAddressTruncated)
    );
}

#[test]
fn encoder_capacity_failures_do_not_mutate_output() {
    let mut short = [0xa5; 5];
    assert_eq!(
        encode_bvll(&mut short, BvlcFunction::OriginalUnicastNpdu, &[1, 2],),
        Err(BipDecodeError::OutputTooSmall {
            needed: 6,
            available: 5,
        })
    );
    assert_eq!(short, [0xa5; 5]);

    let too_long_npdu = vec![0; u16::MAX as usize - 3];
    let mut untouched = [0x5a; 8];
    assert_eq!(
        encode_bvll(
            &mut untouched,
            BvlcFunction::OriginalUnicastNpdu,
            &too_long_npdu,
        ),
        Err(BipDecodeError::FrameTooLong(u16::MAX as usize + 1))
    );
    assert_eq!(untouched, [0x5a; 8]);
}

#[test]
fn encoder_accepts_largest_representable_bvll_datagram() {
    let npdu = vec![0x5a; u16::MAX as usize - 4];
    let mut output = vec![0; u16::MAX as usize];
    let len = encode_bvll(&mut output, BvlcFunction::OriginalUnicastNpdu, &npdu).unwrap();
    assert_eq!(len, u16::MAX as usize);
    assert_eq!(&output[..4], &[0x81, 0x0a, 0xff, 0xff]);
    assert_eq!(
        decode_bip_datagram(&output, meta(false))
            .unwrap()
            .npdu
            .len(),
        npdu.len()
    );
}
