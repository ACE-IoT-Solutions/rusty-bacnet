use super::*;
use bacnet_mstp_codec::{
    encode_extended_into, Frame, FrameHeader, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA,
    MAX_STANDARD_FRAME_LENGTH,
};
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

fn encode_host_extended_frame(source: u8, fill: u8, data_len: usize) -> Vec<u8> {
    let header = FrameHeader::new(
        bacnet_mstp_codec::FrameType::ExtendedDataNotExpectingReply,
        BROADCAST_MAC,
        source,
        data_len,
    );
    let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH];
    let used = encode_extended_into(&mut wire, &header, &vec![fill; data_len]).unwrap();
    wire[..used].to_vec()
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

#[test]
fn mixed_standard_and_extended_frames_are_assembled_in_order() {
    let standard_before = encode_host_data_frame(1, 0x11, 4);
    let extended = encode_host_extended_frame(2, 0x22, 502);
    let standard_after = encode_host_data_frame(3, 0x33, 4);
    let mut chunk = standard_before;
    chunk.extend_from_slice(&extended);
    chunk.extend_from_slice(&standard_after);
    let mut frame_buf = Vec::new();
    let mut scratch = vec![0u8; MAX_EXTENDED_MPDU_DATA];
    let frames = assemble_host_chunk_with_scratch(&mut frame_buf, &chunk, &mut scratch);
    assert_eq!(frames.len(), 3);
    assert_eq!(
        frames[0].frame_type,
        LegacyFrameType::BACnetDataNotExpectingReply
    );
    assert_eq!(frames[0].source, 1);
    assert_eq!(
        frames[1].frame_type,
        LegacyFrameType::ExtendedDataNotExpectingReply
    );
    assert_eq!(frames[1].source, 2);
    assert_eq!(frames[1].data.len(), 502);
    assert_eq!(
        frames[2].frame_type,
        LegacyFrameType::BACnetDataNotExpectingReply
    );
    assert_eq!(frames[2].source, 3);
    assert!(frame_buf.is_empty());
}

#[test]
fn maximum_extended_frame_reassembles_at_every_chunk_boundary() {
    let wire = encode_host_extended_frame(7, 0xA5, MAX_EXTENDED_MPDU_DATA);
    for split in 0..=wire.len() {
        let mut frame_buf = Vec::new();
        let mut scratch = vec![0u8; MAX_EXTENDED_MPDU_DATA];
        let first = assemble_host_chunk_with_scratch(&mut frame_buf, &wire[..split], &mut scratch);
        let frames = if split == wire.len() {
            first
        } else {
            assert!(first.is_empty(), "split={split}");
            assemble_host_chunk_with_scratch(&mut frame_buf, &wire[split..], &mut scratch)
        };
        assert_eq!(frames.len(), 1, "split={split}");
        assert_eq!(frames[0].data.len(), MAX_EXTENDED_MPDU_DATA);
        assert!(frame_buf.is_empty(), "split={split}");
    }
}

#[test]
fn reserved_extended_types_are_rejected_and_next_frame_survives() {
    let invalid = {
        let fields = [0x22, BROADCAST_MAC, 1, 0, 0];
        let mut wire = vec![0x55, 0xFF];
        wire.extend_from_slice(&fields);
        wire.push(bacnet_mstp_codec::crc8(&fields));
        wire
    };
    let valid = encode_host_data_frame(4, 0x44, 3);
    let mut chunk = invalid;
    chunk.extend_from_slice(&valid);
    let mut frame_buf = Vec::new();
    let frames = assemble_host_chunk(&mut frame_buf, &chunk);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].source, 4);
    assert!(frame_buf.is_empty());
}

#[tokio::test]
async fn loopback_mixed_stream_delivers_standard_after_extended_frame() {
    let (serial_a, serial_b) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(
        serial_a,
        MstpConfig {
            this_station: 3,
            ..MstpConfig::default()
        },
    );
    let mut received = transport.start().await.unwrap();

    let extended = encode_host_extended_frame(2, 0x22, 502);
    let standard = encode_host_data_frame(9, 0x99, 4);
    let mut mixed = extended;
    mixed.extend_from_slice(&standard);
    serial_b.write(&mixed).await.unwrap();

    let npdu = tokio::time::timeout(tokio::time::Duration::from_secs(1), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(npdu.source_mac, bacnet_types::MacAddr::from_slice(&[9]));
    assert_eq!(npdu.npdu, Bytes::from(vec![0x99; 4]));
    transport.stop().await.unwrap();
}
