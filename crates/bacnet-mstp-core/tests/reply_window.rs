//! Reply timing for a received data-expecting-reply frame: the decision
//! deadline keeps a transmit margin inside T_reply_delay, and a late deadline
//! call or transmit start still answers with Reply Postponed (Clause 9.5.6.9).

#![cfg(feature = "heapless")]

use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, RequestId, TransmitId,
};

type Queues = HeaplessQueues<4, 4>;

/// The request arrives at 1 ms.
const REQUEST_AT: Instant = Instant::from_micros(1_000);
/// T_reply_delay (250 ms) less the default 25 ms margin, after the request.
const DECIDE_BY: Instant = Instant::from_micros(226_000);
/// The end of T_reply_delay.
const SEND_BY: Instant = Instant::from_micros(251_000);
/// The shortest T_reply_timeout (255 ms) after the request.
const GIVE_UP: Instant = Instant::from_micros(256_000);

fn config() -> MstpCoreConfig {
    MstpCoreConfig {
        station: 1,
        max_master: 3,
        ..MstpCoreConfig::default()
    }
}

/// Deliver a request from station 7 and return its identifier.
fn receive_request(core: &mut MasterCore<'_, Queues>) -> RequestId {
    core.on_validated_frame(
        Frame::new(FrameType::BACnetDataExpectingReply, 1, 7, &[1, 0x04]),
        REQUEST_AT,
    )
    .unwrap();
    match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    }
}

/// Take the queued transmit and check it is a Reply Postponed to station 7.
fn take_postponed(core: &mut MasterCore<'_, Queues>) -> TransmitId {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { id, frame } => {
            let header = decode_frame(frame).unwrap().0.header;
            assert_eq!(
                (header.frame_type, header.destination),
                (FrameType::ReplyPostponed, 7)
            );
            id
        }
        other => panic!("expected transmit, got {other:?}"),
    }
}

#[test]
fn decision_deadline_leaves_the_transmit_margin() {
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    assert_eq!(core.next_deadline(), Some(DECIDE_BY));

    // Platform latency inside the margin is not late.
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, SEND_BY).unwrap();
    core.on_transmit_complete(id, SEND_BY + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    let counters = core.counters();
    assert_eq!(
        (
            counters.deadline_misses,
            counters.late_replies,
            counters.frames_transmitted
        ),
        (0, 0, 1)
    );

    // The margin is configurable.
    let mut rx = [0; 64];
    let config = MstpCoreConfig {
        reply_transmit_margin: Duration::from_millis(50),
        ..config()
    };
    let mut core = MasterCore::new(config, Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    assert_eq!(
        core.next_deadline(),
        Some(SEND_BY - Duration::from_millis(50))
    );
}

#[test]
fn deadline_call_one_microsecond_late_still_sends_postponed() {
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    let late = DECIDE_BY + Duration::from_micros(1);
    core.on_deadline(late).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, late).unwrap();
    assert_eq!(core.counters().deadline_misses, 1);
    assert_eq!(core.counters().late_replies, 0);

    // A deadline call even past T_reply_delay still answers, and the late
    // start is counted too.
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    let late = SEND_BY + Duration::from_micros(1);
    core.on_deadline(late).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, late).unwrap();
    assert_eq!(core.counters().deadline_misses, 1);
    assert_eq!(core.counters().late_replies, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

#[test]
fn transmit_start_200_microseconds_late_still_sends_postponed() {
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    let start = SEND_BY + Duration::from_micros(200);
    core.on_transmit_started(id, start).unwrap();
    assert_eq!(core.counters().late_replies, 1);
    core.on_transmit_complete(id, start + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().frames_transmitted, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

#[test]
fn nothing_is_sent_once_the_requester_may_have_stopped_waiting() {
    // A Reply Postponed that would start at the requester's shortest timeout
    // is refused: the requester may be transmitting again.
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    assert_eq!(
        core.on_transmit_started(id, GIVE_UP),
        Err(CoreError::RequestExpired)
    );
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.counters().late_replies, 0);

    // One microsecond earlier it still goes out, counted late.
    let mut rx = [0; 64];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, GIVE_UP - Duration::from_micros(1))
        .unwrap();
    assert_eq!(core.counters().late_replies, 1);
    assert_eq!(core.counters().expired_requests, 0);
}
