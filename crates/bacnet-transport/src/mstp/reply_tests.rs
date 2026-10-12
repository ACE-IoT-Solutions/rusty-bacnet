//! Answering a Data Expecting Reply: the application's answer, Reply
//! Postponed, and the turnaround before either (Clause 9.5.6.9).

use bacnet_mstp_codec::{FrameType, MAX_STANDARD_MPDU_DATA};
use bytes::Bytes;
use tokio::time::{Duration, Instant};

use super::test_support::{config, Bus};

/// T_reply_delay less the core's default reply transmit margin (25 ms):
/// when the core stops waiting for the application.
const DECISION: Duration = Duration::from_millis(225);
const TICK: Duration = Duration::from_millis(1);

const REQUEST: &[u8] = &[1, 0x04, 0x10];

/// Station 7 asks station 3 for a reply.
async fn requested(bus: &mut Bus) -> (Instant, tokio::sync::oneshot::Sender<Bytes>) {
    let at = bus.send(FrameType::BACnetDataExpectingReply, 3, 7, REQUEST);
    let npdu = bus.npdus.recv().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), REQUEST);
    assert!(!npdu.link_layer_group);
    (
        at,
        npdu.reply_tx.expect("a request carries a reply channel"),
    )
}

/// An answer that is ready goes out as soon as T_turnaround has passed,
/// without waiting for T_reply_delay; a dropped channel gets Reply
/// Postponed at once instead.
#[tokio::test(start_paused = true)]
async fn a_ready_answer_or_a_dropped_channel_goes_out_after_turnaround() {
    for ready in [true, false] {
        let mut bus = Bus::start(config(3, 127)).await;
        let (at, reply) = requested(&mut bus).await;
        if ready {
            reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
        } else {
            drop(reply);
        }
        let sent = bus.next().await;
        assert_eq!(sent.destination, 7);
        if ready {
            assert_eq!(sent.frame_type, FrameType::BACnetDataNotExpectingReply);
            assert_eq!(sent.data, [1, 0, 0x30]);
        } else {
            assert_eq!(sent.frame_type, FrameType::ReplyPostponed);
        }
        assert!(sent.start >= at + bus.turnaround);
        assert!(sent.start <= at + bus.turnaround + TICK);
        bus.stop().await;
    }
}

/// An answer that comes after the turnaround has passed goes out at once.
#[tokio::test(start_paused = true)]
async fn a_later_answer_is_not_held_for_another_turnaround() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(10)).await;
    let ready = Instant::now();
    reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert!(sent.start <= ready + TICK);
    bus.until(sent.end + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.dner_tx_direct, 1);
    bus.stop().await;
}

/// Without an answer the core sends Reply Postponed at its decision
/// deadline, inside T_reply_delay, and closes the channel: a late answer
/// fails at the sender, which then sends it the ordinary way.
#[tokio::test(start_paused = true)]
async fn no_answer_gets_reply_postponed_before_the_reply_delay() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::ReplyPostponed, 7)
    );
    assert!(sent.start >= at + DECISION);
    assert!(sent.start <= at + DECISION + TICK);
    assert!(sent.start < at + Duration::from_millis(250));
    tokio::task::yield_now().await;
    assert!(reply.send(Bytes::from_static(&[1, 0, 0x30])).is_err());
    bus.stop().await;
}

/// A second request while answering the first is dropped (the station can
/// answer one at a time, Clause 9.5.6.9) and does not move the first one's
/// deadline.
#[tokio::test(start_paused = true)]
async fn a_second_request_does_not_extend_the_first_ones_deadline() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, _reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(100)).await;
    bus.send(FrameType::BACnetDataExpectingReply, 3, 8, &[1, 4, 0x20]);
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::ReplyPostponed, 7)
    );
    assert!(sent.start <= at + DECISION + TICK);
    assert!(bus.npdus.try_recv().is_err(), "the second request went up");
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.discarded_while_busy, 1);
    bus.stop().await;
}

/// Other traffic heard while the answer is being prepared moves the
/// answer's turnaround: it starts T_turnaround after that traffic.
#[tokio::test(start_paused = true)]
async fn later_traffic_moves_the_answers_turnaround() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(40)).await;
    let heard = bus.send(FrameType::BACnetDataNotExpectingReply, 9, 8, &[1, 0, 0x20]);
    reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert!(sent.start >= heard + bus.turnaround);
    assert!(sent.start <= heard + bus.turnaround + TICK);
    bus.stop().await;
}

/// An answer too long for a standard frame cannot be sent: the request is
/// postponed instead, and the answer counted as oversize.
#[tokio::test(start_paused = true)]
async fn an_oversized_answer_is_postponed_and_counted() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (_at, reply) = requested(&mut bus).await;
    reply
        .send(Bytes::from(vec![0x55; MAX_STANDARD_MPDU_DATA + 1]))
        .unwrap();
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::ReplyPostponed, 7)
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.outbound_oversize, 1);
    assert_eq!(counts.dner_tx_direct, 0);
    bus.stop().await;
}

/// An answer handed over at the decision deadline either makes it (sent as
/// the reply) or finds Reply Postponed already chosen; then it is not lost
/// but queued, and goes out with the station's next token.
#[tokio::test(start_paused = true)]
async fn an_answer_at_the_deadline_is_never_lost() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.until(token.end + bus.turnaround).await;
    let at = bus.send(FrameType::BACnetDataExpectingReply, 3, 4, REQUEST);
    let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    bus.until(at + DECISION).await;
    let accepted = reply.send(Bytes::from_static(&[1, 0, 0x31])).is_ok();
    let first = bus.next().await;
    assert_eq!(first.destination, 4);
    if first.frame_type == FrameType::ReplyPostponed && accepted {
        bus.until(first.end + bus.turnaround).await;
        bus.send(FrameType::Token, 3, 4, &[]);
        let late = bus.next().await;
        assert_eq!(late.frame_type, FrameType::BACnetDataNotExpectingReply);
        assert_eq!(late.data, [1, 0, 0x31]);
    } else if accepted {
        assert_eq!(first.frame_type, FrameType::BACnetDataNotExpectingReply);
    }
    bus.stop().await;
}
