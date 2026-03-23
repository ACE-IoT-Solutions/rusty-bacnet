//! TransportPort trait for BACnet data-link transport abstraction.
//!
//! MAC addresses are opaque byte slices whose format depends on the transport:
//! - BACnet/IP (Annex J): 6 bytes (4-byte IPv4 + 2-byte port, big-endian)
//! - BACnet/Ethernet (Clause 7): 6 bytes (IEEE 802 MAC)
//! - MS/TP (Clause 9): 1 byte (station address 0-254)

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

/// Optional metadata about the BVLL transport framing.
///
/// Populated by BACnet/IP transports so upper layers can see how an NPDU
/// was delivered (original unicast/broadcast, forwarded via BBMD, etc.).
#[derive(Debug, Clone, Default)]
pub struct TransportMeta {
    /// BVLC function code (e.g. 0x0a Original-Unicast, 0x0b Original-Broadcast,
    /// 0x04 Forwarded-NPDU, 0x06 Distribute-Broadcast-To-Network).
    pub bvlc_function: Option<u8>,
    /// For Forwarded-NPDU: the originating device's IPv4 address.
    pub forwarded_from_ip: Option<[u8; 4]>,
    /// For Forwarded-NPDU: the originating device's port.
    pub forwarded_from_port: Option<u16>,
}

/// A received NPDU from the transport layer.
pub struct ReceivedNpdu {
    /// Raw NPDU bytes (NPDU header + APDU/network-message payload).
    pub npdu: Bytes,
    /// Source MAC address in transport-native format.
    pub source_mac: MacAddr,
    /// Optional reply channel for MS/TP DataExpectingReply frames.
    /// When present, the application layer should send the reply NPDU bytes
    /// through this channel instead of via normal send_unicast.
    pub reply_tx: Option<oneshot::Sender<Bytes>>,
    /// Optional BVLL transport metadata (BACnet/IP only).
    pub transport_meta: Option<TransportMeta>,
}

impl Clone for ReceivedNpdu {
    fn clone(&self) -> Self {
        Self {
            npdu: self.npdu.clone(),
            source_mac: self.source_mac.clone(),
            reply_tx: None, // oneshot::Sender is not Clone; clones lose the reply channel
            transport_meta: self.transport_meta.clone(),
        }
    }
}

impl std::fmt::Debug for ReceivedNpdu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedNpdu")
            .field("npdu", &self.npdu)
            .field("source_mac", &self.source_mac)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(Sender)"))
            .field("transport_meta", &self.transport_meta)
            .finish()
    }
}

/// Trait for BACnet data-link transports.
///
/// Implementations handle the data-link framing (e.g., BVLL for BACnet/IP)
/// and expose a simple send/receive interface for NPDU bytes.
pub trait TransportPort: Send + Sync {
    /// Start the transport. Returns a receiver for incoming NPDUs.
    ///
    /// The transport spawns a background receive task that decodes incoming
    /// frames and sends `ReceivedNpdu` through the returned channel.
    fn start(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;

    /// Stop the transport and clean up resources.
    fn stop(&mut self) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// Send NPDU bytes to a specific MAC address (unicast).
    fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// Broadcast NPDU bytes on the local network.
    fn send_broadcast(
        &self,
        npdu: &[u8],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// This transport's local MAC address.
    fn local_mac(&self) -> &[u8];

    /// Maximum APDU length this transport supports.
    /// BIP/SC: 1476 (default), MS/TP: 480.
    fn max_apdu_length(&self) -> u16 {
        1476
    }
}
