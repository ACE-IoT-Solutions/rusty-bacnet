use super::*;
use bacnet_mstp_codec::{Frame, MAX_STANDARD_FRAME_LENGTH};
use bytes::{Bytes, BytesMut};

use crate::mstp_frame::{encode_frame, FrameType as LegacyFrameType, MAX_STANDARD_MPDU_DATA};

fn encode_host_data_frame(source: u8, fill: u8, data_len: usize) -> Vec<u8> {
    let frame = MstpFrame {
        frame_type: LegacyFrameType::BACnetDataNotExpectingReply,
        destination: 3,
        source,
        data: Bytes::from(vec![fill; data_len]),
    };
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &frame).unwrap();
    wire.to_vec()
}

#[test]
fn compatibility_queue_mirror_handles_class_reordering_without_rejection() {
    let mut legacy = MasterNode::new(MstpConfig {
        this_station: 3,
        ..MstpConfig::default()
    })
    .unwrap();
    mirror_legacy_enqueue(&mut legacy, 7, Bytes::from_static(&[1, 0, 1]));
    mirror_legacy_enqueue(&mut legacy, BROADCAST_MAC, Bytes::from_static(&[1, 0, 2]));

    let mut wire = [0; MAX_STANDARD_FRAME_LENGTH];
    let broadcast = Frame::new(
        bacnet_mstp_codec::FrameType::BACnetDataNotExpectingReply,
        BROADCAST_MAC,
        3,
        &[1, 0, 2],
    );
    let length = broadcast.encode_into(&mut wire).unwrap();
    mirror_legacy_transmit(&mut legacy, &wire[..length]);
    assert_eq!(legacy.tx_queue.len(), 1);
    assert_eq!(legacy.tx_queue.front().unwrap().0, 7);

    for value in 0..=super::super::MAX_TX_QUEUE_DEPTH {
        mirror_legacy_enqueue(&mut legacy, 7, Bytes::from(vec![value as u8]));
    }
    assert_eq!(legacy.tx_queue.len(), super::super::MAX_TX_QUEUE_DEPTH);
}

#[test]
fn drains_coalesced_frames_larger_than_one_frame() {
    let first = encode_host_data_frame(1, 0xA1, 300);
    let second = encode_host_data_frame(2, 0xB2, 300);
    let mut chunk = first;
    chunk.extend_from_slice(&second);
    let mut frame_buf = Vec::new();
    let frames = assemble_host_chunk(&mut frame_buf, &chunk);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].source, 1);
    assert_eq!(frames[1].source, 2);
    assert!(frame_buf.is_empty());
}

#[test]
fn bounds_malformed_input_and_retains_max_partial() {
    let malformed = vec![0xAA; MSTP_MAX_FRAME_BUF * 4 + 17];
    let mut frame_buf = Vec::new();
    assert!(assemble_host_chunk(&mut frame_buf, &malformed).is_empty());
    assert!(frame_buf.len() <= MSTP_MAX_FRAME_BUF);
    let wire = encode_host_data_frame(1, 0xCC, MAX_STANDARD_MPDU_DATA);
    let split = wire.len() - 1;
    assert!(assemble_host_chunk(&mut frame_buf, &wire[..split]).is_empty());
    let frames = assemble_host_chunk(&mut frame_buf, &wire[split..]);
    assert_eq!(frames.len(), 1);
    assert!(frame_buf.is_empty());
    let token = MstpFrame {
        frame_type: LegacyFrameType::Token,
        destination: 3,
        source: 1,
        data: Bytes::new(),
    };
    let mut token_wire = BytesMut::new();
    encode_frame(&mut token_wire, &token).unwrap();
    assert!(
        assemble_host_chunk(&mut frame_buf, &[0xAA, crate::mstp_frame::PREAMBLE[0]]).is_empty()
    );
    assert_eq!(frame_buf, crate::mstp_frame::PREAMBLE[..1]);
    let frames = assemble_host_chunk(&mut frame_buf, &token_wire[1..]);
    assert_eq!(frames, vec![token]);
}
