//! Type-erased transport for mixed-transport routing.
//!
//! [`AnyTransport`] wraps all supported BACnet transport types, enabling
//! a single router to manage heterogeneous ports (e.g., BIP + MS/TP).

use bacnet_types::error::Error;
use tokio::sync::mpsc;

use crate::bip::BipTransport;
#[cfg(feature = "ipv6")]
use crate::bip6::Bip6Transport;
use crate::loopback::LoopbackTransport;
use crate::mstp::{MstpTransport, SerialPort};
use crate::port::{DataAttribute, ReceivedNpdu, TransportPort};
use crate::virtual_network::VirtualNetwork;

#[cfg(all(feature = "ethernet", target_os = "linux"))]
use crate::ethernet::EthernetTransport;

#[cfg(feature = "sc-tls")]
use crate::sc::ScTransport;
#[cfg(feature = "sc-tls")]
use crate::sc_tls::TlsWebSocket;

/// A transport that can be any supported BACnet transport type.
///
/// Enables mixed-transport routing (e.g., BIP + MS/TP on the same router).
pub enum AnyTransport<S: SerialPort + 'static> {
    /// BACnet/IP over UDP.
    Bip(BipTransport),
    /// MS/TP over RS-485.
    Mstp(MstpTransport<S>),
    /// BACnet/IPv6 over UDP.
    #[cfg(feature = "ipv6")]
    Bip6(Bip6Transport),
    /// BACnet Ethernet over raw LLC frames (Linux only).
    #[cfg(all(feature = "ethernet", target_os = "linux"))]
    Ethernet(EthernetTransport),
    /// BACnet/SC over TLS WebSocket.
    #[cfg(feature = "sc-tls")]
    Sc(Box<ScTransport<TlsWebSocket>>),
    /// In-process loopback (for gateway client/server composition).
    Loopback(LoopbackTransport),
    /// Named in-process N-node virtual network.
    Virtual(VirtualNetwork),
}

impl<S: SerialPort + 'static> TransportPort for AnyTransport<S> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        match self {
            Self::Bip(t) => t.start().await,
            Self::Mstp(t) => t.start().await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.start().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.start().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.start().await,
            Self::Loopback(t) => t.start().await,
            Self::Virtual(t) => t.start().await,
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.stop().await,
            Self::Mstp(t) => t.stop().await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.stop().await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.stop().await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.stop().await,
            Self::Loopback(t) => t.stop().await,
            Self::Virtual(t) => t.stop().await,
        }
    }

    fn abort(&mut self) {
        match self {
            Self::Bip(t) => t.abort(),
            Self::Mstp(t) => t.abort(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.abort(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.abort(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.abort(),
            Self::Loopback(t) => t.abort(),
            Self::Virtual(t) => t.abort(),
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_unicast(npdu, mac).await,
            Self::Mstp(t) => t.send_unicast(npdu, mac).await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.send_unicast(npdu, mac).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_unicast(npdu, mac).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_unicast(npdu, mac).await,
            Self::Loopback(t) => t.send_unicast(npdu, mac).await,
            Self::Virtual(t) => t.send_unicast(npdu, mac).await,
        }
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Mstp(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Loopback(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Virtual(t) => {
                t.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(t) => t.send_broadcast(npdu).await,
            Self::Mstp(t) => t.send_broadcast(npdu).await,
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.send_broadcast(npdu).await,
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.send_broadcast(npdu).await,
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.send_broadcast(npdu).await,
            Self::Loopback(t) => t.send_broadcast(npdu).await,
            Self::Virtual(t) => t.send_broadcast(npdu).await,
        }
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Mstp(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Loopback(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Virtual(t) => {
                t.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
        }
    }

    fn local_mac(&self) -> &[u8] {
        match self {
            Self::Bip(t) => t.local_mac(),
            Self::Mstp(t) => t.local_mac(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.local_mac(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.local_mac(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.local_mac(),
            Self::Loopback(t) => t.local_mac(),
            Self::Virtual(t) => t.local_mac(),
        }
    }

    fn bip_network_port_observation(&self) -> Option<crate::port::BipNetworkPortObservation> {
        match self {
            Self::Bip(transport) => transport.bip_network_port_observation(),
            _ => None,
        }
    }

    fn max_apdu_length(&self) -> u16 {
        match self {
            Self::Bip(t) => t.max_apdu_length(),
            Self::Mstp(t) => t.max_apdu_length(),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.max_apdu_length(),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.max_apdu_length(),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.max_apdu_length(),
            Self::Loopback(t) => t.max_apdu_length(),
            Self::Virtual(t) => t.max_apdu_length(),
        }
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        match self {
            Self::Bip(t) => t.is_broadcast_mac(mac),
            Self::Mstp(t) => t.is_broadcast_mac(mac),
            #[cfg(feature = "ipv6")]
            Self::Bip6(t) => t.is_broadcast_mac(mac),
            #[cfg(all(feature = "ethernet", target_os = "linux"))]
            Self::Ethernet(t) => t.is_broadcast_mac(mac),
            #[cfg(feature = "sc-tls")]
            Self::Sc(t) => t.is_broadcast_mac(mac),
            Self::Loopback(t) => t.is_broadcast_mac(mac),
            Self::Virtual(t) => t.is_broadcast_mac(mac),
        }
    }
}

impl<S: SerialPort> From<BipTransport> for AnyTransport<S> {
    fn from(t: BipTransport) -> Self {
        Self::Bip(t)
    }
}

impl<S: SerialPort> From<MstpTransport<S>> for AnyTransport<S> {
    fn from(t: MstpTransport<S>) -> Self {
        Self::Mstp(t)
    }
}

#[cfg(feature = "ipv6")]
impl<S: SerialPort> From<Bip6Transport> for AnyTransport<S> {
    fn from(t: Bip6Transport) -> Self {
        Self::Bip6(t)
    }
}

#[cfg(all(feature = "ethernet", target_os = "linux"))]
impl<S: SerialPort> From<EthernetTransport> for AnyTransport<S> {
    fn from(t: EthernetTransport) -> Self {
        Self::Ethernet(t)
    }
}

#[cfg(feature = "sc-tls")]
impl<S: SerialPort> From<ScTransport<TlsWebSocket>> for AnyTransport<S> {
    fn from(t: ScTransport<TlsWebSocket>) -> Self {
        Self::Sc(Box::new(t))
    }
}

impl<S: SerialPort> From<LoopbackTransport> for AnyTransport<S> {
    fn from(t: LoopbackTransport) -> Self {
        Self::Loopback(t)
    }
}

impl<S: SerialPort> From<VirtualNetwork> for AnyTransport<S> {
    fn from(t: VirtualNetwork) -> Self {
        Self::Virtual(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mstp::{LoopbackSerial, MstpConfig};
    use std::net::Ipv4Addr;

    #[test]
    fn any_transport_bip_local_mac() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(bip);
        assert_eq!(any.local_mac().len(), 6);
    }

    #[test]
    fn any_transport_bip_max_apdu() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(bip);
        assert_eq!(any.max_apdu_length(), 1476);
    }

    #[test]
    fn any_transport_delegates_read_only_bip_network_port_observation() {
        let plain: AnyTransport<LoopbackSerial> = AnyTransport::Bip(BipTransport::new(
            Ipv4Addr::LOCALHOST,
            47808,
            Ipv4Addr::BROADCAST,
        ));
        assert!(plain.bip_network_port_observation().is_some());
        assert!(plain
            .bip_network_port_observation()
            .unwrap()
            .bbmd()
            .is_none());

        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        bbmd.enable_bbmd(Vec::new());
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip(bbmd);
        let reader = any.bip_network_port_observation().unwrap().bbmd().unwrap();
        assert_eq!(reader.lifecycle(), crate::bip::BbmdLifecycle::NotStarted);
    }

    #[test]
    fn any_transport_mstp_local_mac() {
        let (serial, _) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 42,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mstp = MstpTransport::new(serial, config);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Mstp(mstp);
        assert_eq!(any.local_mac(), &[42]);
    }

    #[test]
    fn any_transport_mstp_max_apdu() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Mstp(mstp);
        assert_eq!(any.max_apdu_length(), 480);
        assert!(any.bip_network_port_observation().is_none());
    }

    #[test]
    fn any_transport_from_bip() {
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::BROADCAST);
        let any: AnyTransport<LoopbackSerial> = bip.into();
        assert_eq!(any.max_apdu_length(), 1476);
    }

    #[test]
    fn any_transport_from_mstp() {
        let (serial, _) = LoopbackSerial::pair();
        let mstp = MstpTransport::new(serial, MstpConfig::default());
        let any: AnyTransport<LoopbackSerial> = mstp.into();
        assert_eq!(any.max_apdu_length(), 480);
    }

    #[tokio::test]
    async fn any_transport_virtual_delegates_current_contract() {
        let network_name = "any-transport-virtual-current-contract";
        let mut sender: AnyTransport<LoopbackSerial> =
            VirtualNetwork::join(network_name, 41).unwrap().into();
        let mut receiver: AnyTransport<LoopbackSerial> =
            VirtualNetwork::join(network_name, 42).unwrap().into();

        assert_eq!(sender.local_mac(), &[41]);
        assert_eq!(sender.max_apdu_length(), 1476);
        assert!(!sender.is_broadcast_mac(&[0xff]));

        let _sender_rx = sender.start().await.unwrap();
        let mut receiver_rx = receiver.start().await.unwrap();
        let attributes = [DataAttribute {
            option_type: 1,
            must_understand: false,
            data: vec![7],
        }];

        sender
            .send_unicast_with_data_attributes(b"unicast", &[42], &attributes)
            .await
            .unwrap();
        let received = receiver_rx.recv().await.unwrap();
        assert_eq!(received.npdu, bytes::Bytes::from_static(b"unicast"));
        assert!(!received.link_layer_group);
        assert!(received.data_attributes.is_empty());

        sender
            .send_broadcast_with_data_attributes(b"broadcast", &attributes)
            .await
            .unwrap();
        let received = receiver_rx.recv().await.unwrap();
        assert_eq!(received.npdu, bytes::Bytes::from_static(b"broadcast"));
        assert!(received.link_layer_group);
        assert!(received.data_attributes.is_empty());

        sender.stop().await.unwrap();
        receiver.abort();
        let replacement = VirtualNetwork::join(network_name, 42).unwrap();
        drop(replacement);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_bip6_local_mac() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.local_mac().len(), 18);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_bip6_max_apdu() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = AnyTransport::Bip6(bip6);
        assert_eq!(any.max_apdu_length(), 1476);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn any_transport_from_bip6() {
        let bip6 = crate::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 47808, None);
        let any: AnyTransport<LoopbackSerial> = bip6.into();
        assert_eq!(any.max_apdu_length(), 1476);
    }
}
