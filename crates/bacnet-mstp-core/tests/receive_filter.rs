//! Which received frames end WAIT_FOR_REPLY, and which dropped candidates
//! count as invalid frames for this station (Clauses 9.5.4, 9.5.6.4, 9.5.8).

#![cfg(feature = "heapless")]

use bacnet_mstp_codec::{crc8, Frame, FrameError, FrameType, BROADCAST_MAC, PREAMBLE};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, NetworkPriority,
};

type Queues = HeaplessQueues<4, 4>;
type Actions = bacnet_mstp_core::ActionBuffers;

const STATION: u8 = 1;
/// The station this one sends its request to.
const ASKED: u8 = 3;
/// Another station's address.
const OTHER: u8 = 9;

fn config() -> MstpCoreConfig {
    MstpCoreConfig {
        station: STATION,
        max_master: 3,
        ..MstpCoreConfig::default()
    }
}

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = [0u8; 600];
    let length = frame.encode_into(&mut wire).unwrap();
    wire[..length].to_vec()
}

/// A header with a correct CRC, for field values encode refuses.
fn raw_header(frame_type: u8, destination: u8, source: u8, length: u16) -> Vec<u8> {
    let [high, low] = length.to_be_bytes();
    let fields = [frame_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    wire
}

/// Send a data-expecting-reply frame to `ASKED` and wait for its reply.
fn wait_for_reply(core: &mut MasterCore<'_, Queues>) -> Instant {
    core.enqueue_npdu(ASKED, &[1, 0x04], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(Frame::new(FrameType::Token, STATION, 0, &[]), Instant::ZERO)
        .unwrap();
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { id, .. } = core.poll_action() else {
        panic!("expected the request transmit");
    };
    core.on_transmit_started(id, due).unwrap();
    let done = due + Duration::from_millis(1);
    core.on_transmit_complete(id, done).unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::WaitForReply);
    done
}

/// Feed `wire` octet by octet, 100 µs apart, and return the last octet's time.
fn feed(core: &mut MasterCore<'_, Queues>, wire: &[u8], start: Instant) -> Instant {
    let mut now = start;
    for &octet in wire {
        now += Duration::from_micros(100);
        core.on_octet(octet, now).unwrap();
    }
    now
}

/// While waiting for a reply, feed `wire` through receive storage of
/// `storage` octets and report whether it ended the wait as an invalid frame.
/// An invalid frame ends this token use, which here polls for a master
/// (Clause 9.5.6.4); a frame ignored as another station's leaves the wait
/// running.
fn counted_invalid(storage: usize, wire: &[u8]) -> bool {
    let mut rx = vec![0u8; storage];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let start = wait_for_reply(&mut core);
    feed(&mut core, wire, start);
    let counters = core.counters();
    assert_eq!(counters.frames_received, 1, "only the token was valid");
    match (core.state(), counters.parser_errors) {
        // Octets still arriving after the invalid header cancel the poll
        // while it waits out the turnaround, which leaves the core idle.
        (MasterState::PollForMaster | MasterState::Idle, 1) => true,
        (MasterState::WaitForReply, 0) => false,
        other => panic!("unexpected outcome {other:?}"),
    }
}

#[test]
fn a_reply_type_frame_from_any_station_ends_the_wait() {
    for (frame_type, source) in [
        (FrameType::BACnetDataNotExpectingReply, 5),
        (FrameType::ReplyPostponed, 4),
        (FrameType::TestResponse, 6),
    ] {
        let mut rx = [0u8; 64];
        let mut actions = Actions::new();
        let mut core = MasterCore::new(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        let now = wait_for_reply(&mut core);
        let data: &[u8] = if frame_type.allows_data() { &[7] } else { &[] };
        core.on_validated_frame(Frame::new(frame_type, STATION, source, data), now)
            .unwrap();
        // DONE_WITH_TOKEN, which polls for a successor here, not IDLE.
        assert_eq!(core.state(), MasterState::PollForMaster, "{frame_type:?}");
        if frame_type == FrameType::BACnetDataNotExpectingReply {
            assert_eq!(
                core.poll_action(),
                Action::DeliverNpdu {
                    source,
                    destination: STATION,
                    npdu: &[7],
                    request_id: None,
                }
            );
        }
    }

    // A reply addressed elsewhere is an unexpected frame: back to IDLE.
    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let now = wait_for_reply(&mut core);
    core.on_validated_frame(
        Frame::new(FrameType::BACnetDataNotExpectingReply, 2, ASKED, &[7]),
        now,
    )
    .unwrap();
    assert_eq!(core.state(), MasterState::Idle);
}

#[test]
fn validated_frame_the_codec_would_refuse_changes_nothing() {
    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let late = Instant::from_micros(10_000);
    let data = [0u8; 502];
    assert_eq!(
        core.on_validated_frame(
            Frame::new(FrameType::BACnetDataNotExpectingReply, STATION, 5, &data),
            late,
        ),
        Err(CoreError::Frame(FrameError::Oversize {
            length: 502,
            maximum: 501,
        }))
    );
    assert_eq!(
        core.on_validated_frame(Frame::new(FrameType::Token, STATION, 255, &[]), late),
        Err(CoreError::Frame(FrameError::BroadcastSource))
    );
    assert_eq!(
        core.on_validated_frame(Frame::new(FrameType::Unknown(0x20), STATION, 5, &[1]), late),
        Err(CoreError::Frame(FrameError::UnsupportedCobsFrame {
            raw_type: 0x20
        }))
    );
    assert_eq!(core.counters().frames_received, 0);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.next_deadline(), Some(Instant::from_micros(500_000)));
    // The refused frames did not move the core's clock either.
    core.on_silence(Instant::from_micros(1)).unwrap();
}

#[test]
fn a_frame_too_long_for_storage_counts_only_for_this_station() {
    // 40 data octets make a 50-octet frame; the storage holds 32.
    let frame = |destination| {
        encode(Frame::new(
            FrameType::BACnetDataNotExpectingReply,
            destination,
            5,
            &[0; 40],
        ))
    };
    assert!(!counted_invalid(32, &frame(OTHER)));
    assert!(counted_invalid(32, &frame(STATION)));
    assert!(counted_invalid(32, &frame(BROADCAST_MAC)));
}

#[test]
fn a_reserved_cobs_frame_counts_only_for_this_station() {
    // Reserved COBS type 34 with a Length of 506, inside the COBS range: a
    // good header of a type the codec doesn't decode. The codec would skip
    // the rest of the frame by count; here it never comes.
    let header = |destination| raw_header(0x22, destination, 5, 506);
    assert!(!counted_invalid(600, &header(OTHER)));
    assert!(counted_invalid(600, &header(STATION)));
    assert!(counted_invalid(600, &header(BROADCAST_MAC)));
}

/// The core doesn't take extended frames yet: the codec decodes them, but
/// the core drops one its storage holds as an unsupported type, and one its
/// storage can't hold as too long. Either way it counts only when it is for
/// this station or broadcast, as a COBS frame did before the codec decoded
/// them.
#[test]
fn an_extended_frame_counts_only_for_this_station() {
    // 100 data octets make a 114-octet frame.
    let frame = |destination| {
        encode(Frame::new(
            FrameType::BACnetExtendedDataNotExpectingReply,
            destination,
            5,
            &[0x11; 100],
        ))
    };
    assert_eq!(frame(OTHER).len(), 114);
    for storage in [64, 600] {
        assert!(!counted_invalid(storage, &frame(OTHER)), "{storage}");
        assert!(counted_invalid(storage, &frame(STATION)), "{storage}");
        assert!(counted_invalid(storage, &frame(BROADCAST_MAC)), "{storage}");
    }
}

#[test]
fn a_bad_data_crc_counts_only_for_this_station() {
    let frame = |destination| {
        let mut wire = encode(Frame::new(
            FrameType::BACnetDataNotExpectingReply,
            destination,
            5,
            &[1, 2, 3],
        ));
        wire[9] ^= 0x01;
        wire
    };
    assert!(!counted_invalid(600, &frame(OTHER)));
    assert!(counted_invalid(600, &frame(STATION)));
    assert!(counted_invalid(600, &frame(BROADCAST_MAC)));
}

#[test]
fn a_bad_header_counts_whatever_its_destination() {
    let mut bad_crc = encode(Frame::new(FrameType::Token, OTHER, 5, &[]));
    bad_crc[7] ^= 0x01;
    assert!(counted_invalid(600, &bad_crc), "header CRC");
    // Source 255 (Clause 9.3).
    assert!(
        counted_invalid(600, &raw_header(0x06, OTHER, 255, 0)),
        "source"
    );
    // A Length over 501 on a type that isn't COBS-encoded.
    assert!(
        counted_invalid(600, &raw_header(0x06, OTHER, 5, 600)),
        "length"
    );
    // A COBS header with source 255, or a Length outside the COBS range
    // (Clause 9.5.8): the codec checks both before the type now, so neither
    // passes for a good header of a type it doesn't decode (#1611).
    assert!(
        counted_invalid(600, &raw_header(0x22, OTHER, 255, 506)),
        "COBS source"
    );
    for length in [4, 2044] {
        assert!(
            counted_invalid(600, &raw_header(0x20, OTHER, 5, length)),
            "COBS length {length}"
        );
    }
}

#[test]
fn the_decoder_is_drained_after_a_dropped_candidate() {
    // A header whose CRC fails, with a Token's first octets in its last
    // three. The codec drops the first octet and rescans the rest, finding
    // the Token's start.
    let fields = [0x06, OTHER, 5, 0x55, 0xFF];
    assert_ne!(crc8(&fields), 0x00);
    let mut bad = PREAMBLE.to_vec();
    bad.extend_from_slice(&fields);
    bad.push(0x00);
    let token = encode(Frame::new(FrameType::Token, STATION, 2, &[]));
    assert_eq!(token[..3], bad[5..]);

    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let last = feed(&mut core, &bad, Instant::ZERO);
    assert_eq!(core.counters().parser_errors, 1);
    // Draining at once leaves the Token's partial header current, so its
    // frame-abort deadline runs from this octet.
    let abort = core.config().t_frame_abort;
    assert_eq!(
        core.next_deadline(),
        Some(last + abort + Duration::from_micros(1))
    );
    feed(&mut core, &token[3..], last);
    assert_eq!(core.counters().frames_received, 1);
    // The Token was for this station; with nothing queued it polls for a
    // successor.
    assert_eq!(core.state(), MasterState::PollForMaster);
}
