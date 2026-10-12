//! A router's MS/TP ports take extended frames (#1611): Clause 9.3 requires
//! COBS-encoded frames of a routing node, so `BACnetRouter::start` turns
//! them on for every MS/TP port it routes, whatever its configuration says.

use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::TransportPort;
use tokio::time::{timeout, Duration};

use super::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

/// A station of masters 0 and 1 at 38400 baud, without extended frames.
fn standard(station: u8) -> MstpConfig {
    MstpConfig {
        this_station: station,
        max_master: 1,
        baud_rate: 38_400,
        ..MstpConfig::default()
    }
}

/// The router's two MS/TP ports are configured without extended frames. A
/// peer on network 1 that takes them broadcasts a 1000-octet NPDU, which
/// goes out as a Type 33; the router's port takes it and delivers the APDU
/// locally. A port without extended frames would count the frame as invalid
/// and deliver nothing.
#[tokio::test(start_paused = true)]
async fn a_routers_mstp_ports_take_extended_frames() {
    let (router_serial_1, peer_serial_1) = LoopbackSerial::pair();
    let (router_serial_2, _peer_serial_2) = LoopbackSerial::pair();
    let ports = vec![
        RouterPort {
            transport: MstpTransport::new(router_serial_1, standard(0)),
            network_number: 1,
        },
        RouterPort {
            transport: MstpTransport::new(router_serial_2, standard(0)),
            network_number: 2,
        },
    ];
    let StartedRouter {
        mut router,
        mut apdus,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();

    let mut peer = MstpTransport::new(
        peer_serial_1,
        MstpConfig {
            extended_frames: true,
            ..standard(1)
        },
    );
    let _from_router = peer.start().await.unwrap();
    // A local broadcast Who-Is, padded past what a standard frame carries.
    let mut npdu = vec![0x01, 0x00, 0x10, 0x08];
    npdu.resize(1000, 0);
    peer.send_broadcast(&npdu).await.unwrap();

    let received = timeout(Duration::from_secs(10), apdus.recv())
        .await
        .expect("the router delivers the broadcast")
        .unwrap();
    assert_eq!(received.apdu.as_ref(), &npdu[2..]);
    assert_eq!(received.ingress_network, Some(1));
    assert_eq!(received.source_mac.as_slice(), &[1]);
    router.stop().await;
    peer.stop().await.unwrap();
}
