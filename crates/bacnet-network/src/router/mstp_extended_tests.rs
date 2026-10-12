//! A router's MS/TP ports take extended frames (#1611): Clause 9.3 requires
//! COBS-encoded frames of a routing node, so `BACnetRouter::start` turns
//! them on for every MS/TP port it routes, whatever its configuration says.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::{GroupDestinations, ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use tokio::sync::mpsc;
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

/// An MS/TP port that records the APDU limits it reports once started,
/// after the router has prepared it.
struct Watched {
    inner: MstpTransport<LoopbackSerial>,
    /// `local_receive_apdu_capacity` and `egress_apdu_limit` at start.
    limits: Arc<[AtomicU16; 2]>,
}

impl TransportPort for Watched {
    fn prepare_for_routing(&mut self) {
        self.inner.prepare_for_routing();
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let received = self.inner.start().await?;
        self.limits[0].store(self.inner.local_receive_apdu_capacity(), Ordering::SeqCst);
        self.limits[1].store(self.inner.egress_apdu_limit(), Ordering::SeqCst);
        Ok(received)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }

    fn egress_apdu_limit(&self) -> u16 {
        self.inner.egress_apdu_limit()
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }

    fn group_destinations(&self) -> GroupDestinations {
        self.inner.group_destinations()
    }
}

/// The router forwards a 1000-octet NPDU from network 2 to station 1 on
/// network 1. Its MS/TP port, configured without extended frames, sends it
/// in an extended frame, since no standard frame carries more than 501
/// octets, and both ports report the 1476-octet APDU limit once the router
/// has prepared them.
#[tokio::test(start_paused = true)]
async fn a_routers_mstp_port_sends_extended_frames() {
    let (router_serial_1, peer_serial_1) = LoopbackSerial::pair();
    let (router_serial_2, peer_serial_2) = LoopbackSerial::pair();
    let limits: Vec<Arc<[AtomicU16; 2]>> = (0..2)
        .map(|_| Arc::new([AtomicU16::new(0), AtomicU16::new(0)]))
        .collect();
    let ports = vec![
        RouterPort {
            transport: Watched {
                inner: MstpTransport::new(router_serial_1, standard(0)),
                limits: limits[0].clone(),
            },
            network_number: 1,
        },
        RouterPort {
            transport: Watched {
                inner: MstpTransport::new(router_serial_2, standard(0)),
                limits: limits[1].clone(),
            },
            network_number: 2,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    for port in &limits {
        assert_eq!(port[0].load(Ordering::SeqCst), 1476);
        assert_eq!(port[1].load(Ordering::SeqCst), 1476);
    }

    let extended = |station| MstpConfig {
        extended_frames: true,
        ..standard(station)
    };
    let mut destination = MstpTransport::new(peer_serial_1, extended(1));
    let mut from_router = destination.start().await.unwrap();
    let mut source = MstpTransport::new(peer_serial_2, extended(1));
    let _to_source = source.start().await.unwrap();

    // To DNET 1, DADR 1, hop count 255: a Who-Is padded past what a
    // standard frame carries.
    let mut apdu = vec![0x10, 0x08];
    apdu.resize(990, 0x5A);
    let mut npdu = vec![0x01, 0x20, 0x00, 0x01, 0x01, 0x01, 0xFF];
    npdu.extend_from_slice(&apdu);
    source.send_unicast(&npdu, &[0]).await.unwrap();

    // Skip the router's own network messages, such as I-Am-Router-To-Network.
    let forwarded = timeout(Duration::from_secs(10), async {
        loop {
            let received = from_router.recv().await.unwrap();
            if received
                .npdu
                .get(1)
                .is_some_and(|control| control & 0x80 == 0)
            {
                break received;
            }
        }
    })
    .await
    .expect("the router forwards the NPDU");
    assert!(forwarded.npdu.len() > 501, "{}", forwarded.npdu.len());
    assert!(forwarded.npdu.ends_with(&apdu));
    assert_eq!(forwarded.source_mac.as_slice(), &[0]);
    router.stop().await;
    destination.stop().await.unwrap();
    source.stop().await.unwrap();
}
