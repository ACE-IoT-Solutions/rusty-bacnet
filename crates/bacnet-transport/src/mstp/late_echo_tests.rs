//! Echo that arrives short, late or split across the grace. The transport
//! must never drop a peer's octets, and never feed its own echo to the core
//! after the frame's completion, where it would read as another station's
//! traffic.

use std::sync::atomic::Ordering;

use bacnet_mstp_codec::FrameType;
use tokio::time::Duration;

use super::test_support::{config, encode, Bus, DrainMode, Port, Sent};
use super::Echo;
use crate::port::TransportPort;

const T_USAGE_TIMEOUT: Duration = Duration::from_millis(20);
const TICK: Duration = Duration::from_millis(1);
const GRACE: Duration = Duration::from_millis(5);

/// A port whose echo is due within 5 ms; it echoes until a test turns that
/// off to script the echo itself.
const SHORT_GRACE: Port = Port {
    echo: Echo::Full { grace: GRACE },
    echoes: true,
    drain: DrainMode::Immediate,
};

/// The wire octets of a frame the station sent.
fn octets(sent: &Sent) -> Vec<u8> {
    encode(sent.frame_type, sent.destination, sent.source, &sent.data)
}

/// Station 3 in a ring with station 4, joined with the echo on, then left
/// to the test to echo. Returns the Token it passed on joining.
async fn ring(port: Port) -> (Bus, Sent) {
    let mut bus = Bus::start_with(config(3, 4), port).await;
    let token = bus.join(4).await;
    bus.echoing.store(false, Ordering::SeqCst);
    (bus, token)
}

/// A short echo, missing its first octets, followed at once by the peer's
/// reply in the same read: the echo cannot be matched, so all of it reaches
/// the core (as noise), and the reply after it is still a reply. WAIT_FOR_REPLY
/// takes it and passes the token (ReceivedReply, Clause 9.5.6.4).
#[tokio::test(start_paused = true)]
async fn a_short_echo_does_not_swallow_the_reply_behind_it() {
    let (mut bus, token) = ring(Port {
        echo: Echo::full(),
        ..SHORT_GRACE
    })
    .await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACnetDataExpectingReply);
    bus.until(request.end + TICK).await;
    let mut chunk = octets(&request)[3..].to_vec();
    chunk.extend(encode(
        FrameType::BACnetDataNotExpectingReply,
        3,
        4,
        &[1, 0, 0x20],
    ));
    let heard = bus.send_raw(&chunk);
    let reply = bus.npdus.recv().await.unwrap();
    assert_eq!(reply.npdu.as_ref(), &[1, 0, 0x20]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::Token, 4));
    assert!(token.start >= heard + bus.turnaround);
    assert!(token.start <= heard + bus.turnaround + TICK);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_mismatches, 1);
    assert_eq!(counts.wait_for_reply_timeouts, 0);
    assert_eq!(counts.mac.own_address_frames, 0);
    bus.stop().await;
}

/// With no echo at all, a peer's answer inside the grace starts the way
/// the station's frame did (the preamble); those octets are held, then
/// released with the rest at the first difference, so the answer arrives
/// whole: the polled station becomes the successor (ReceivedReplyToPFM,
/// Clause 9.5.6.8).
#[tokio::test(start_paused = true)]
async fn a_peer_frame_where_the_echo_should_be_reaches_the_core_whole() {
    let mut bus = Bus::start_with(
        config(3, 7),
        Port {
            echo: Echo::full(),
            echoes: false,
            drain: DrainMode::Immediate,
        },
    )
    .await;
    bus.send(FrameType::Token, 3, 6, &[]);
    let poll = bus.next().await;
    assert_eq!(poll.destination, 4);
    bus.until(poll.end + bus.turnaround).await;
    bus.send(FrameType::ReplyToPollForMaster, 3, 4, &[]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::Token, 4));
    bus.stop().await;
}

/// An echo that arrives entirely after the grace, while the station waits
/// in PASS_TOKEN: the frame was completed at the grace, but the filter still
/// recognizes the echo and drops it, so it is not taken for the successor
/// using the token (SawTokenUser, Clause 9.5.6.6) and the station retries.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_pass_token_is_not_the_successor() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    assert_eq!(passed.frame_type, FrameType::Token);
    bus.until(passed.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&octets(&passed));
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::Token, 4));
    assert!(retry.start >= passed.end + T_USAGE_TIMEOUT);
    assert!(retry.start <= passed.end + T_USAGE_TIMEOUT + TICK);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_missing, 1);
    assert_eq!(counts.echo_mismatches, 0);
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.token_retries, 1);
    bus.stop().await;
}

/// A late echo in WAIT_FOR_REPLY is dropped too: fed, it would be an
/// unexpected frame and drop the token (Clause 9.5.6.4). The reply after it
/// ends the wait, and the station passes the token.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_wait_for_reply_keeps_the_token() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACnetDataExpectingReply);
    bus.until(request.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&octets(&request));
    bus.until(request.end + Duration::from_millis(15)).await;
    let heard = bus.send(FrameType::BACnetDataNotExpectingReply, 3, 4, &[1, 0, 0x20]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::Token, 4));
    assert!(token.start <= heard + bus.turnaround + TICK, "{token:?}");
    assert_eq!(bus.npdus.recv().await.unwrap().npdu.as_ref(), &[1, 0, 0x20]);
    assert_eq!(
        bus.transport
            .diagnostics()
            .snapshot()
            .mac
            .own_address_frames,
        0
    );
    bus.stop().await;
}

/// A late echo in POLL_FOR_MASTER is dropped too: fed, it would be an
/// unexpected frame and drop the token (Clause 9.5.6.8). The station polls on
/// after T_usage_timeout.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_poll_for_master_keeps_the_token() {
    let mut bus = Bus::start_with(
        config(3, 7),
        Port {
            echoes: false,
            ..SHORT_GRACE
        },
    )
    .await;
    bus.send(FrameType::Token, 3, 6, &[]);
    let poll = bus.next().await;
    assert_eq!(poll.destination, 4);
    bus.until(poll.end + GRACE + Duration::from_millis(3)).await;
    bus.send_raw(&octets(&poll));
    let next = bus.next().await;
    assert_eq!(
        (next.frame_type, next.destination),
        (FrameType::PollForMaster, 5)
    );
    assert!(next.start >= poll.end + T_USAGE_TIMEOUT);
    assert!(next.start <= poll.end + T_USAGE_TIMEOUT + TICK);
    bus.stop().await;
}

/// The grace runs out half way through the echo: the frame is completed
/// then, and the rest of the echo, arriving later, is still dropped.
#[tokio::test(start_paused = true)]
async fn the_rest_of_an_echo_cut_by_the_grace_is_still_dropped() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    let echo = octets(&passed);
    bus.until(passed.end + TICK).await;
    bus.send_raw(&echo[..4]);
    bus.until(passed.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&echo[4..]);
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::Token, 4));
    assert!(retry.start >= passed.end + T_USAGE_TIMEOUT);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!((counts.echo_missing, counts.echo_mismatches), (0, 0));
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.token_retries, 1);
    bus.stop().await;
}
