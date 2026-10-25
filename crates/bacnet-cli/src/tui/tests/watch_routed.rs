//! The watch list against the lab device behind a router (#1790): routed
//! rows subscribe by COV, follow the device's changes, are renewed through
//! the router, and are cancelled on quit.

use std::time::Duration;

use bacnet_server::LocalCommandSource;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;

use super::site::Site;
use super::watch::{ai, command, options, settle, sources};
use crate::tui::watched::Source;

const LAB: u32 = 100;

fn active(site: &Site) -> u64 {
    site.routed_server().cov_counters().subscriptions_active
}

/// Both kinds of routed row show COV, a change on the device reaches its
/// row, the subscriptions outlive two lifetimes on renewals sent through
/// the router, and quitting leaves the device's COV table empty with no
/// renewal after.
#[tokio::test(start_paused = true)]
async fn routed_rows_subscribe_renew_and_cancel_through_the_router() {
    let mut site = Site::routed(LAB, options(60, 10, 0)).await;
    site.global_who_is("", 1).await;
    let device = site.app.devices.rows().next().unwrap();
    assert_eq!(device.network, Some(super::site::ROUTED_NETWORK));
    command(&mut site, "watch 2:100 ai:1").await;
    command(&mut site, "watch 2:100 ai:2 status-flags").await;
    settle(&mut site).await;
    assert_eq!(
        sources(&site),
        [Source::Cov { lifetime: 60 }, Source::Cov { lifetime: 60 }]
    );
    site.run_for(Duration::from_secs(1)).await;
    assert_eq!(active(&site), 2);
    assert_eq!(site.app.watch.stats.subscriptions, 2);

    site.routed_server()
        .write_local(
            &ai(1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(77.0),
            None,
            LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    site.run_for(Duration::from_secs(1)).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.value.as_ref().unwrap().as_deref(), Ok("77 °F"));

    site.run_for(Duration::from_secs(130)).await;
    assert_eq!(active(&site), 2, "renewals kept both subscriptions");
    assert_eq!(
        sources(&site),
        [Source::Cov { lifetime: 60 }, Source::Cov { lifetime: 60 }]
    );

    site.quit().await;
    assert_eq!(active(&site), 0, "the cancels reached the device");
    tokio::time::sleep(Duration::from_secs(200)).await;
    assert_eq!(active(&site), 0, "no renewal outlived the session");
    site.stop().await;
}
