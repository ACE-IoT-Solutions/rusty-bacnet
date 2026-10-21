//! A peer's flood of APDUs that don't decode, or of segments the server
//! refuses, warns once per interval for each kind, with the occurrences
//! since the last warning; the rest log at debug (#1950).

use super::*;
use crate::server::warn_spy::{WarnSpy, Warning};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::warn_throttle::WARNING_INTERVAL;
use bacnet_types::enums::Segmentation;
use request_reassembly::{recv_apdu, send_segment_with_window, start_reassembly_server};

const UNDECODABLE: &str = "Server failed to decode received APDU";
const NO_SEGMENT_0: &str = "Received non-initial segment without prior segment 0, aborting";

/// Send `count` APDUs of PDU type 15, which doesn't exist, then `count`
/// segments numbered 1 with no segment 0 before them, and wait for each
/// segment's Abort, which also shows the APDUs before it were handled.
async fn flood(
    client: &LoopbackTransport,
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    count: u8,
) {
    for _ in 0..count {
        let npdu = [0x01, 0x00, 0xF0];
        client.send_unicast(&npdu, &[0x02]).await.unwrap();
    }
    for invoke_id in 0..count {
        send_segment_with_window(client, invoke_id, 1, 1, true, &[0]).await;
        let Apdu::Abort(abort) = recv_apdu(rx, "segment without segment 0").await else {
            panic!("expected an Abort");
        };
        assert_eq!(abort.invoke_id, invoke_id);
    }
}

fn warned(spy: &WarnSpy, message: &str) -> Vec<Option<u64>> {
    spy.warnings()
        .into_iter()
        .filter(|warning: &Warning| warning.message == message)
        .map(|warning| warning.occurrences)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_flood_of_bad_apdus_and_refused_segments_warns_once_per_interval() {
    let (spy, _guard) = WarnSpy::install();
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    flood(&client, &mut rx, 5).await;
    assert_eq!(warned(&spy, UNDECODABLE), [Some(1)]);
    assert_eq!(warned(&spy, NO_SEGMENT_0), [Some(1)]);

    tokio::time::advance(WARNING_INTERVAL).await;
    flood(&client, &mut rx, 3).await;
    assert_eq!(
        warned(&spy, UNDECODABLE),
        [Some(1), Some(5)],
        "four held back, and the first of the next burst"
    );
    assert_eq!(warned(&spy, NO_SEGMENT_0), [Some(1), Some(5)]);
    // Nothing else the floods set off warned (the other is the startup
    // warning about the default vendor identifier).
    assert_eq!(spy.warnings().len(), 5, "{:?}", spy.warnings());
    drop(server);
}
