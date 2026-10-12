//! `send_apdu` to a group address of the link (#1677, L22).
//!
//! With no DNET, a group address is the link's local broadcast, so an
//! Unconfirmed-Request to one goes out through `send_broadcast`: B/IP
//! refuses a unicast to a group address. Anything else goes to the
//! transport as given, where B/IP refuses it.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use super::*;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::mpsc;

/// A Who-Is, an Unconfirmed-Request.
const WHO_IS: [u8; 2] = [0x10, 0x08];
/// A ReadProperty request, invoke ID 1.
const CONFIRMED: [u8; 4] = [0x00, 0x05, 0x01, 0x0C];

/// A send the layer asked of the transport.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sent {
    Unicast { mac: Vec<u8>, npdu: Vec<u8> },
    Broadcast { npdu: Vec<u8> },
}

/// A B/IP transport whose sends are recorded. A unicast goes on to it, so
/// its refusals show; a broadcast stops here, so no test datagram leaves
/// the host.
struct Recorded {
    inner: BipTransport,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl TransportPort for Recorded {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let sent = Sent::Unicast {
            mac: mac.to_vec(),
            npdu: npdu.to_vec(),
        };
        self.sent.lock().unwrap().push(sent);
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let sent = Sent::Broadcast {
            npdu: npdu.to_vec(),
        };
        self.sent.lock().unwrap().push(sent);
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.inner.is_group_destination(mac)
    }

    fn group_destinations(&self) -> GroupDestinations {
        self.inner.group_destinations()
    }
}

#[tokio::test]
async fn an_unconfirmed_request_to_a_group_address_goes_out_as_a_local_broadcast() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let inner = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(192, 0, 2, 255));
    let mut net = NetworkLayer::new(Recorded {
        inner,
        sent: Arc::clone(&sent),
    });
    let _rx = net.start().await.unwrap();
    let port = net.transport().inner.local_mac()[4..].to_vec();
    let at = |ip: [u8; 4]| [&ip[..], &port].concat();
    let p = NetworkPriority::NORMAL;
    let groups = [
        at([192, 0, 2, 255]),
        at([255, 255, 255, 255]),
        at([239, 1, 2, 3]),
    ];
    for group in &groups {
        net.send_apdu(&WHO_IS, group, false, p).await.unwrap();
    }
    let attributes = [DataAttribute {
        option_type: 7,
        must_understand: false,
        data: vec![1],
    }];
    net.send_apdu_with_data_attributes(&WHO_IS, &groups[0], false, p, &attributes)
        .await
        .unwrap();
    // One device's address still takes a unicast.
    let peer = at([127, 0, 0, 1]);
    net.send_apdu(&WHO_IS, &peer, false, p).await.unwrap();
    // A confirmed request goes to the transport as given, which refuses it.
    let refused = net.send_apdu(&CONFIRMED, &groups[0], true, p).await;
    let error = refused.unwrap_err();
    assert!(error.to_string().contains("group address"), "{error}");

    let local_broadcast = [&[0x01, 0x00][..], &WHO_IS].concat();
    let mut expected = vec![
        Sent::Broadcast {
            npdu: local_broadcast.clone()
        };
        groups.len() + 1
    ];
    expected.push(Sent::Unicast {
        mac: peer,
        npdu: local_broadcast,
    });
    expected.push(Sent::Unicast {
        mac: groups[0].clone(),
        npdu: [&[0x01, 0x04][..], &CONFIRMED].concat(),
    });
    assert_eq!(*sent.lock().unwrap(), expected);
    net.stop().await.unwrap();
}

/// The rule is taken again when the transport starts and knows its own
/// address: on a loopback link whose broadcast IP is its own, the broadcast
/// MAC is that IP at the port bound at start, which the transport didn't
/// know before.
#[tokio::test]
async fn the_group_rule_follows_the_port_bound_at_start() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let inner = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut net = NetworkLayer::new(Recorded {
        inner,
        sent: Arc::clone(&sent),
    });
    let _rx = net.start().await.unwrap();
    let broadcast = net.transport().inner.local_mac().to_vec();
    assert!(net.transport().is_broadcast_mac(&broadcast));
    net.send_apdu(&WHO_IS, &broadcast, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
    let local_broadcast = [&[0x01, 0x00][..], &WHO_IS].concat();
    assert_eq!(
        *sent.lock().unwrap(),
        [Sent::Broadcast {
            npdu: local_broadcast
        }]
    );
    net.stop().await.unwrap();
}
