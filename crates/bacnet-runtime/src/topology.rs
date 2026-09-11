use crate::{AttachmentId, RouterObservation, RuntimeError};

/// One explicit BBMD seed for a topology walk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BbmdTarget {
    /// Attachment used to reach the BBMD.
    pub attachment_id: AttachmentId,
    /// Six-byte B/IP MAC, including the exact UDP port.
    pub mac: Vec<u8>,
}

/// Coarse B/IP topology request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologyRequest {
    /// Attachments to query; empty means all B/IP attachments.
    pub attachment_ids: Vec<AttachmentId>,
    /// Initial BBMDs. BDT peers are walked recursively.
    pub bbmd_seeds: Vec<BbmdTarget>,
    /// Read FDT state for every reachable BBMD.
    pub include_fdt: bool,
    /// Per-attachment safety bound for recursively discovered BBMDs.
    pub max_bbmds_per_attachment: usize,
}

impl Default for TopologyRequest {
    fn default() -> Self {
        Self {
            attachment_ids: Vec::new(),
            bbmd_seeds: Vec::new(),
            include_fdt: true,
            max_bbmds_per_attachment: 256,
        }
    }
}

/// One BDT peer with exact wire address and broadcast mask.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BdtRecord {
    /// IPv4 address bytes.
    pub ip: [u8; 4],
    /// UDP port.
    pub port: u16,
    /// Broadcast mask bytes.
    pub broadcast_mask: [u8; 4],
}

impl BdtRecord {
    pub(crate) fn mac(&self) -> Vec<u8> {
        let mut mac = Vec::with_capacity(6);
        mac.extend_from_slice(&self.ip);
        mac.extend_from_slice(&self.port.to_be_bytes());
        mac
    }
}

/// One FDT registration with live wire expiry state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FdtRecord {
    /// Foreign-device IPv4 bytes.
    pub ip: [u8; 4],
    /// Foreign-device UDP port.
    pub port: u16,
    /// Registered TTL in seconds.
    pub ttl: u16,
    /// Seconds remaining reported by the BBMD.
    pub seconds_remaining: u16,
}

/// Tables read from one reachable BBMD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BbmdSnapshot {
    /// Exact BBMD B/IP MAC queried.
    pub mac: Vec<u8>,
    /// Broadcast Distribution Table.
    pub bdt: Vec<BdtRecord>,
    /// Foreign Device Table, empty when not requested.
    pub fdt: Vec<FdtRecord>,
}

/// Topology owned by one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentTopology {
    /// Owning attachment.
    pub attachment_id: AttachmentId,
    /// Exact local transport MAC.
    pub local_mac: Vec<u8>,
    /// Reachable BBMD tables in deterministic walk order.
    pub bbmds: Vec<BbmdSnapshot>,
    /// Latest router observations retained by the client.
    pub routers: Vec<RouterObservation>,
    /// True when the BBMD safety bound stopped the walk.
    pub truncated: bool,
}

/// Aggregated runtime topology snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeTopology {
    /// Runtime generation used for the query.
    pub generation: u64,
    /// Per-attachment topology in configuration/request order.
    pub attachments: Vec<AttachmentTopology>,
    /// Partial per-attachment/BBMD failures.
    pub errors: Vec<RuntimeError>,
}
