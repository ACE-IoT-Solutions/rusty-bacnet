#![cfg(feature = "heapless")]

use core::mem::size_of;

use bacnet_mstp_codec::{
    encode_extended_into, ExtendedFrame, Frame, FrameHeader, FrameType, MAX_EXTENDED_FRAME_LENGTH,
    MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA,
};
use bacnet_mstp_core::{
    Action, ExtendedMasterCore, HeaplessQueues, Instant, NetworkPriority, QueueStorage,
    StandardMasterCore,
};
#[cfg(feature = "alloc")]
use bacnet_mstp_core::{ExtendedAllocQueues, QueueRejectReason};

type StandardQueues = HeaplessQueues<2, 2, MAX_STANDARD_MPDU_DATA, 1>;
type ExtendedQueues = HeaplessQueues<2, 2, MAX_EXTENDED_MPDU_DATA, 1>;

fn at(millis: u64) -> Instant {
    Instant::from_micros(millis * 1_000)
}

fn extended_frame<'a>(
    wire: &'a mut [u8; MAX_EXTENDED_FRAME_LENGTH],
    payload: &[u8],
) -> (ExtendedFrame<'a>, usize) {
    let header = FrameHeader::new(
        FrameType::ExtendedDataNotExpectingReply,
        0,
        7,
        payload.len(),
    );
    let used = encode_extended_into(wire, &header, payload).unwrap();
    let (frame, _) = bacnet_mstp_codec::decode_extended_frame_in_place(&mut wire[..used]).unwrap();
    (frame, used)
}

#[test]
fn extended_profile_delivers_502_byte_npdu_and_standard_rejects_it() {
    let mut payload = [0; MAX_EXTENDED_MPDU_DATA];
    payload[..MAX_STANDARD_MPDU_DATA + 1].fill(0xA5);
    let mut standard_wire = [0; MAX_EXTENDED_FRAME_LENGTH];
    let (standard_frame, _) =
        extended_frame(&mut standard_wire, &payload[..MAX_STANDARD_MPDU_DATA + 1]);

    let mut standard_rx = [0; MAX_EXTENDED_FRAME_LENGTH];
    let mut standard = StandardMasterCore::new(
        Default::default(),
        StandardQueues::new(),
        &mut standard_rx,
        Instant::ZERO,
    )
    .unwrap();
    assert!(matches!(
        standard.on_validated_extended_frame(standard_frame, at(1)),
        Err(bacnet_mstp_core::CoreError::ExtendedFrameUnsupported { .. })
    ));
    assert_eq!(standard.counters().npdus_delivered, 0);

    let mut extended_rx = [0; MAX_EXTENDED_FRAME_LENGTH];
    let mut extended = ExtendedMasterCore::new(
        Default::default(),
        ExtendedQueues::new(),
        &mut extended_rx,
        Instant::ZERO,
    )
    .unwrap();
    let mut extended_wire = [0; MAX_EXTENDED_FRAME_LENGTH];
    let (extended_frame, _) =
        extended_frame(&mut extended_wire, &payload[..MAX_STANDARD_MPDU_DATA + 1]);
    extended
        .on_validated_extended_frame(extended_frame, at(1))
        .unwrap();
    assert!(matches!(
        extended.poll_action(),
        Action::DeliverNpdu { npdu, .. } if npdu.len() == MAX_STANDARD_MPDU_DATA + 1
    ));
}

#[test]
fn profile_rejects_1498_before_queue_mutation_and_selects_der_frame() {
    let mut rx = [0; MAX_EXTENDED_FRAME_LENGTH];
    let mut core = ExtendedMasterCore::new(
        Default::default(),
        ExtendedQueues::new(),
        &mut rx,
        Instant::ZERO,
    )
    .unwrap();
    let oversized = [0x11; MAX_EXTENDED_MPDU_DATA + 1];
    assert_eq!(
        core.enqueue_npdu(2, &oversized, NetworkPriority::Normal, at(1)),
        Err(bacnet_mstp_core::QueueRejectReason::Oversize)
    );
    assert_eq!(core.counters().unicast.depth, 0);

    let payload = [0x22; MAX_STANDARD_MPDU_DATA + 1];
    core.enqueue_npdu(2, &payload, NetworkPriority::Normal, at(2))
        .unwrap();
    core.on_validated_frame(Frame::new(FrameType::Token, 0, 2, &[]), at(3))
        .unwrap();
    core.on_deadline(at(10)).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(bacnet_mstp_core::Direction::Transmit)
    );
    let action = core.poll_action();
    assert!(matches!(
        action,
        Action::Transmit { frame, .. } if frame[2] == FrameType::ExtendedDataNotExpectingReply.to_raw()
    ));
}

#[test]
fn extended_profile_accepts_incremental_stream_delivery() {
    let payload = [0x5A; MAX_STANDARD_MPDU_DATA + 1];
    let mut wire = [0; MAX_EXTENDED_FRAME_LENGTH];
    let header = FrameHeader::new(
        FrameType::ExtendedDataNotExpectingReply,
        0,
        7,
        payload.len(),
    );
    let used = encode_extended_into(&mut wire, &header, &payload).unwrap();
    let mut rx = [0; MAX_EXTENDED_FRAME_LENGTH];
    let mut core = ExtendedMasterCore::new(
        Default::default(),
        ExtendedQueues::new(),
        &mut rx,
        Instant::ZERO,
    )
    .unwrap();
    for byte in &wire[..used] {
        core.on_octet(*byte, at(1)).unwrap();
    }
    let counters = core.counters();
    let action = core.poll_action();
    assert!(
        matches!(action, Action::DeliverNpdu { npdu, .. } if npdu.len() == MAX_STANDARD_MPDU_DATA + 1),
        "action={action:?}, counters={counters:?}"
    );
}

#[test]
fn representative_profile_storage_delta_is_measured_and_nonzero() {
    let standard = size_of::<StandardMasterCore<'static, StandardQueues>>();
    let extended = size_of::<ExtendedMasterCore<'static, ExtendedQueues>>();
    // The extended profile intentionally pays for larger queue/action slots;
    // the standard profile remains the compact 501-byte default.
    std::println!("MS/TP profile storage: standard={standard} bytes, extended={extended} bytes, delta={} bytes", extended - standard);
    assert!(
        extended > standard,
        "standard={standard}, extended={extended}"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn allocation_extended_queue_uses_the_extended_slot_profile() {
    let mut queues = ExtendedAllocQueues::new(1, 0);
    let payload = [0xC3; MAX_EXTENDED_MPDU_DATA];
    queues
        .enqueue_npdu(1, &payload, NetworkPriority::Normal, 0, None)
        .unwrap();
    assert_eq!(queues.depth(bacnet_mstp_core::QueueKind::Unicast), 1);
    let oversized = [0xD4; MAX_EXTENDED_MPDU_DATA + 1];
    assert_eq!(
        queues.enqueue_npdu(1, &oversized, NetworkPriority::Normal, 0, None),
        Err(QueueRejectReason::Oversize)
    );
}
