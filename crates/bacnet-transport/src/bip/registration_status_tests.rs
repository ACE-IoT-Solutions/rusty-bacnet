//! What a foreign device reports about its registration (#1622): the lease
//! in its snapshot, and one log line each when it becomes registered, is
//! refused, or lapses.

use std::time::Duration;

use tracing::Level;

use super::registration_schedule_tests::Device;
use crate::log_capture::LogCapture;

const REGISTERED: &str = "B/IP foreign device registered";
const REFUSED: &str = "B/IP BBMD refused";
const LAPSED: &str = "B/IP foreign-device registration lapsed";

/// Registered at once, refused at 10 s, then every attempt lost until the
/// lease, proven from 0 s for the 60 s time-to-live, lapses; a success at
/// 65 s registers it again.
#[tokio::test(start_paused = true)]
async fn each_transition_is_logged_once_and_the_lease_follows_it() {
    let (logs, _guard) = LogCapture::start();
    let device = Device::new(Duration::from_secs(10)).await;
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(10_013).await;
    device.bbmd_says(0x30).await;
    device.at(59_999).await;
    let live = device.ctx.node.snapshot().foreign_registration;
    assert!(live.registered);
    assert_eq!(live.lease_remaining, Some(Duration::from_millis(1)));
    device.at(60_500).await;
    let lapsed = device.ctx.node.snapshot().foreign_registration;
    assert!(!lapsed.registered);
    assert_eq!(lapsed.lease_remaining, None);
    assert_eq!(lapsed.lapses, 1);
    // A broadcast with no live lease goes, and is counted.
    device
        .ctx
        .node
        .send_broadcast(&[0x01, 0x00], None)
        .await
        .unwrap();
    assert_eq!(
        device
            .ctx
            .node
            .snapshot()
            .foreign_registration
            .broadcasts_unregistered,
        1
    );
    // The attempt at 65 s is answered.
    device.at(65_013).await;
    device.bbmd_says(0).await;
    assert!(device.ctx.node.snapshot().foreign_registration.registered);

    assert_eq!(logs.levels(REGISTERED), [Level::INFO, Level::INFO]);
    assert_eq!(logs.levels(REFUSED), [Level::WARN]);
    assert_eq!(logs.levels(LAPSED), [Level::WARN]);
    // Nothing else warns: the losses and renewals are routine.
    assert_eq!(logs.warnings().len(), 2, "{:?}", logs.warnings());
}
