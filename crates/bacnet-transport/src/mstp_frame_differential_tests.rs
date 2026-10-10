use super::*;

fn legacy_frame(
    frame_type: FrameType,
    destination: u8,
    source: u8,
    data: &[u8],
) -> legacy::MstpFrame {
    legacy::MstpFrame {
        frame_type: legacy::FrameType::from_raw(frame_type.to_raw()),
        destination,
        source,
        data: Bytes::copy_from_slice(data),
    }
}

fn header_only_wire(frame_type: u8, data_len: usize) -> Vec<u8> {
    let header = [
        frame_type,
        1,
        0,
        (data_len >> 8) as u8,
        (data_len & 0xFF) as u8,
    ];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&header);
    wire.push(crc8(&header));
    wire
}

fn assert_decode_equivalent(wire: &[u8]) {
    let old = legacy::decode_frame(wire);
    let new = decode_frame(wire);
    match (old, new) {
        (Ok((old_frame, old_consumed)), Ok((new_frame, new_consumed))) => {
            assert_eq!(old_consumed, new_consumed);
            assert_eq!(old_frame.frame_type.to_raw(), new_frame.frame_type.to_raw());
            assert_eq!(old_frame.destination, new_frame.destination);
            assert_eq!(old_frame.source, new_frame.source);
            assert_eq!(old_frame.data, new_frame.data);
        }
        (Err(old), Err(new)) => assert_eq!(old.to_string(), new.to_string()),
        (old, new) => panic!("old/new decode decision differs: {old:?} vs {new:?}"),
    }
}

fn assert_stream_equivalent(wire: &[u8]) {
    let old = legacy::decode_frame_stream(wire);
    let new = decode_frame_stream(wire);
    match (old, new) {
        (legacy::StreamDecode::NeedMore, StreamDecode::NeedMore) => {}
        (
            legacy::StreamDecode::Invalid {
                discard: old_discard,
            },
            StreamDecode::Invalid {
                discard: new_discard,
            },
        ) => assert_eq!(old_discard, new_discard),
        (
            legacy::StreamDecode::Complete {
                frame: old_frame,
                consumed: old_consumed,
            },
            StreamDecode::Complete {
                frame: new_frame,
                consumed: new_consumed,
            },
        ) => {
            assert_eq!(old_consumed, new_consumed);
            assert_eq!(old_frame.frame_type.to_raw(), new_frame.frame_type.to_raw());
            assert_eq!(old_frame.destination, new_frame.destination);
            assert_eq!(old_frame.source, new_frame.source);
            assert_eq!(old_frame.data, new_frame.data);
        }
        (old, new) => panic!("old/new stream decision differs: {old:?} vs {new:?}"),
    }
}

#[test]
fn differential_valid_frames_match_legacy_wire_and_decode() {
    let cases = [
        (FrameType::Token, 0, 7, &[][..]),
        (
            FrameType::BACnetDataNotExpectingReply,
            BROADCAST_MAC,
            3,
            &[0x01, 0x00][..],
        ),
        (
            FrameType::BACnetDataExpectingReply,
            5,
            0,
            &[0x01, 0x22, 0x30][..],
        ),
        // Preserve the historical asymmetry: control types may carry payload bytes.
        (FrameType::Token, 1, 0, &[0xAA, 0x55][..]),
    ];

    for (frame_type, destination, source, data) in cases {
        let frame = MstpFrame {
            frame_type,
            destination,
            source,
            data: Bytes::copy_from_slice(data),
        };
        let mut new_wire = BytesMut::new();
        encode_frame(&mut new_wire, &frame).unwrap();
        let mut old_wire = BytesMut::new();
        legacy::encode_frame(
            &mut old_wire,
            &legacy_frame(frame_type, destination, source, data),
        )
        .unwrap();
        assert_eq!(&new_wire[..], &old_wire[..]);
        assert_decode_equivalent(&new_wire);
        assert_stream_equivalent(&new_wire);
    }
}

#[test]
fn differential_malformed_frames_match_legacy_errors_and_stream_discard() {
    let valid = {
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: 1,
            source: 0,
            data: Bytes::from_static(&[0x01, 0x22, 0x30]),
        };
        let mut wire = BytesMut::new();
        encode_frame(&mut wire, &frame).unwrap();
        wire.to_vec()
    };

    let mut bad_preamble = valid.clone();
    bad_preamble[0] = 0;
    let mut bad_header_crc = valid.clone();
    bad_header_crc[7] ^= 0x01;
    let mut bad_source = valid.clone();
    bad_source[4] = 128;
    bad_source[7] = crc8(&bad_source[2..7]);
    let mut broadcast_source = valid.clone();
    broadcast_source[4] = BROADCAST_MAC;
    broadcast_source[7] = crc8(&broadcast_source[2..7]);
    let mut bad_data_crc = valid.clone();
    let last = bad_data_crc.len() - 1;
    bad_data_crc[last] ^= 0x01;
    let mut truncated = valid.clone();
    truncated.truncate(truncated.len() - 1);
    let oversized = header_only_wire(0x06, MAX_STANDARD_MPDU_DATA + 1);

    for wire in [
        bad_preamble,
        bad_header_crc,
        bad_source,
        broadcast_source,
        bad_data_crc,
        truncated,
        oversized,
    ] {
        assert_decode_equivalent(&wire);
        assert_stream_equivalent(&wire);
    }
}

#[test]
fn differential_rejects_every_cobs_frame_type() {
    for raw_type in 0x20..=0x7F {
        let wire = header_only_wire(raw_type, 0);
        assert_decode_equivalent(&wire);
        assert_stream_equivalent(&wire);

        let frame = MstpFrame {
            frame_type: FrameType::Unknown(raw_type),
            destination: 1,
            source: 0,
            data: Bytes::new(),
        };
        let mut new_wire = BytesMut::new();
        let new_result = encode_frame(&mut new_wire, &frame);
        let mut old_wire = BytesMut::new();
        let old_result = legacy::encode_frame(
            &mut old_wire,
            &legacy_frame(frame.frame_type, frame.destination, frame.source, &[]),
        );
        assert_eq!(new_result.is_ok(), old_result.is_ok());
        assert_eq!(new_wire, old_wire);
        if let (Err(old), Err(new)) = (old_result, new_result) {
            assert_eq!(old.to_string(), new.to_string());
        }
    }
}

#[test]
fn differential_crc_and_error_precedence_edges_match_legacy() {
    assert_eq!(crc8_valid(&[0]), legacy::crc8_valid(&[0]));

    let oversized = vec![0; MAX_STANDARD_MPDU_DATA + 1];
    let frame = MstpFrame {
        frame_type: FrameType::Unknown(0x20),
        destination: 1,
        source: 0,
        data: Bytes::copy_from_slice(&oversized),
    };
    let mut new_wire = BytesMut::new();
    let new_error = encode_frame(&mut new_wire, &frame).unwrap_err();
    let mut old_wire = BytesMut::new();
    let old_error = legacy::encode_frame(
        &mut old_wire,
        &legacy_frame(
            frame.frame_type,
            frame.destination,
            frame.source,
            &oversized,
        ),
    )
    .unwrap_err();
    assert_eq!(new_error.to_string(), old_error.to_string());
    assert_eq!(new_wire, old_wire);
}
