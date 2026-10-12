//! A foreign device's registration schedule on the node engine, as it was
//! before the engine (#1622): an attempt every renewal interval, counted
//! from the start of the one before, whatever came of it: an answer, a
//! refusal, a timeout, a busy slot, or a Distribute-Broadcast-To-Network NAK
//! from the BBMD. Each attempt waits for its result for the interval or 3 s,
//! whichever is shorter.

use std::time::Duration;

use bacnet_bip_core::node::Mode;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::send_sink::SendSink;
use super::*;

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0);
const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0);
const BBMD: ([u8; 4], u16) = ([198, 51, 100, 1], 0xBAC0);

/// A foreign device renewing every `interval`, what it sends, and its
/// driver.
struct Device {
    ctx: RecvContext,
    sink: SendSink,
    start: Instant,
    _driver: JoinHandle<()>,
}

impl Device {
    async fn new(interval: Duration) -> Self {
        let sink = SendSink::default();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
        let config = ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::from(BBMD.0),
            bbmd_port: BBMD.1,
            ttl: 60,
            renewal_interval: Some(interval),
        };
        let mode = Mode::Foreign(config.engine(config.interval().unwrap()));
        let link = test_support::test_link(OWN, BROADCAST, mode);
        let (npdu_tx, _) = mpsc::channel(4);
        let ctx = RecvContext::on_link(socket, npdu_tx, link, None);
        let driver = tokio::spawn(Arc::clone(&ctx.node).drive());
        Self {
            ctx,
            sink,
            start: Instant::now(),
            _driver: driver,
        }
    }

    /// Let time run to `ms` milliseconds from the start.
    async fn at(&self, ms: u64) {
        tokio::time::sleep_until(self.start + Duration::from_millis(ms)).await;
    }

    /// The BBMD answers with `code`.
    async fn bbmd_says(&self, code: u16) {
        let result = BvllMessage {
            function: BvlcFunction::BVLC_RESULT,
            payload: bytes::Bytes::copy_from_slice(&code.to_be_bytes()),
        };
        handle_bvll_message(&result, BBMD, Delivery::Unicast, &self.ctx).await;
    }

    /// When each registration attempt went, in milliseconds from the start.
    fn attempts(&self) -> Vec<u128> {
        let bbmd = SocketAddrV4::new(Ipv4Addr::from(BBMD.0), BBMD.1);
        self.sink
            .take()
            .into_iter()
            .filter(|sent| sent.to == bbmd && sent.bytes == [0x81, 0x05, 0x00, 0x06, 0x00, 60])
            .map(|sent| (sent.at - self.start).as_millis())
            .collect()
    }
}

#[tokio::test(start_paused = true)]
async fn attempts_go_one_interval_apart_whatever_came_of_them() {
    let device = Device::new(Duration::from_secs(10)).await;
    // The first goes at once, and is answered.
    device.at(13).await;
    device.bbmd_says(0).await;
    // The second is refused.
    device.at(10_013).await;
    device.bbmd_says(0x30).await;
    // The third goes unanswered, and times out at 23 s; the next still
    // waits for 30 s.
    device.at(23_000).await;
    tokio::task::yield_now().await;
    let lost = device.ctx.node.snapshot();
    assert_eq!(lost.register_foreign_device.timeouts, 1);
    assert_eq!(
        lost.foreign_registration.last_outcome,
        Some(ForeignRegistrationOutcome::TimedOut)
    );
    assert_eq!(
        lost.foreign_registration.next_attempt_in,
        Some(Duration::from_secs(7))
    );
    device.at(30_013).await;
    device.bbmd_says(0).await;
    // A Distribute-Broadcast-To-Network NAK from the BBMD pulls nothing
    // forward.
    device.at(30_050).await;
    device.bbmd_says(0x60).await;
    // A manual request holds the one slot over 40 s: that attempt is busy,
    // and the next waits for 50 s.
    device.at(35_000).await;
    let request = Bvll::ReadBroadcastDistributionTable;
    let peer = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 30), 0xBAC0);
    let held = device.ctx.node.begin(peer, request, Duration::from_secs(3));
    device.at(45_000).await;
    drop(held);
    device.at(50_500).await;

    assert_eq!(device.attempts(), [0, 10_000, 20_000, 30_000, 50_000]);
    let snapshot = device.ctx.node.snapshot();
    let counters = snapshot.register_foreign_device;
    assert_eq!(
        (
            counters.sent,
            counters.results,
            counters.timeouts,
            counters.busy
        ),
        (5, 3, 1, 1)
    );
    let registration = snapshot.foreign_registration;
    assert_eq!(
        (
            registration.attempts,
            registration.accepted,
            registration.refused
        ),
        (5, 2, 1)
    );
    assert_eq!(snapshot.unmatched_responses, 1);
    assert_eq!(
        registration.next_attempt_in,
        Some(Duration::from_millis(9_500))
    );
}

/// An attempt waits for its result for the interval when that is shorter
/// than 3 s, so a lost one ends as the next falls due, and an answer that
/// comes later belongs to the next.
#[tokio::test(start_paused = true)]
async fn a_short_interval_bounds_the_wait_and_a_late_answer_meets_the_next_attempt() {
    let device = Device::new(Duration::from_millis(300)).await;
    // Nothing for the first; the one at 300 ms takes this late success.
    device.at(303).await;
    device.bbmd_says(0).await;
    device.at(950).await;
    assert_eq!(device.attempts(), [0, 300, 600, 900]);
    let snapshot = device.ctx.node.snapshot();
    assert_eq!(snapshot.register_foreign_device.timeouts, 2);
    assert_eq!(snapshot.foreign_registration.accepted, 1);
}
