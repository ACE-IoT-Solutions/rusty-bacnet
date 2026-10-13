//! What the differential test compares, in forms both routers' outputs
//! convert to, and the helpers that read them.

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_router_core::router::Kind;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use super::super::control_policy::ControlGate;
use super::super::legacy;
use super::super::SendRequest;
use super::Arrival;
use crate::layer::{AdmissionReceiver, ReceivedApdu, ReceivedNetworkControl};

/// What a port queue held, comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Wire {
    to: Option<Vec<u8>>,
    npdu: Vec<u8>,
    attributes: Vec<DataAttribute>,
}

/// A delivered APDU, comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Delivered {
    apdu: Vec<u8>,
    source_mac: Vec<u8>,
    ingress_network: Option<u16>,
    source_network: Option<NpduAddress>,
    link_layer_group: bool,
    is_group: bool,
    global_broadcast: bool,
    data_attributes: Vec<DataAttribute>,
    provenance: TransportProvenance,
    direct_response: bool,
    reply: bool,
}

impl From<ReceivedApdu> for Delivered {
    fn from(apdu: ReceivedApdu) -> Self {
        Self {
            apdu: apdu.apdu.to_vec(),
            source_mac: apdu.source_mac.to_vec(),
            ingress_network: apdu.ingress_network,
            source_network: apdu.source_network,
            link_layer_group: apdu.link_layer_group,
            is_group: apdu.is_group,
            global_broadcast: apdu.global_broadcast,
            data_attributes: apdu.data_attributes,
            provenance: apdu.provenance,
            direct_response: apdu.direct_response.is_some(),
            reply: apdu.reply_tx.is_some(),
        }
    }
}

/// A control for the router's consumer, comparable.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Control {
    npdu: Npdu,
    source_mac: Vec<u8>,
    link_layer_group: bool,
    data_attributes: Vec<DataAttribute>,
    provenance: TransportProvenance,
    ingress_sequence: u64,
}

impl From<ReceivedNetworkControl> for Control {
    fn from(control: ReceivedNetworkControl) -> Self {
        Self {
            npdu: control.npdu,
            source_mac: control.source_mac.to_vec(),
            link_layer_group: control.link_layer_group,
            data_attributes: control.data_attributes,
            provenance: control.provenance,
            ingress_sequence: control.ingress_sequence,
        }
    }
}

/// A route, comparable: times in nanoseconds since the clock's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RouteView {
    pub(super) network: u16,
    pub(super) port: usize,
    pub(super) direct: bool,
    pub(super) next_hop: Vec<u8>,
    pub(super) last_seen: Option<u64>,
    pub(super) last_used: Option<u64>,
    /// 0 reachable, 1 busy, 2 unreachable.
    pub(super) reachability: u8,
    pub(super) busy_until: Option<u64>,
    pub(super) flap_count: u8,
    pub(super) last_port_change: Option<u64>,
    pub(super) pending: Option<(usize, u64)>,
}

/// The four drop counts, the claim counts and the rest, as numbers.
pub(super) type Counts = Vec<u64>;

/// The frame `arrival` describes, as a transport hands it over.
pub(super) fn received(arrival: &Arrival) -> ReceivedNpdu {
    let attributes = if arrival.attributes {
        vec![DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }]
    } else {
        Vec::new()
    };
    let reply_tx = arrival.reply.then(|| oneshot::channel().0);
    ReceivedNpdu {
        npdu: Bytes::copy_from_slice(&arrival.npdu),
        source_mac: MacAddr::from_slice(&arrival.source),
        link_layer_group: arrival.link_group,
        data_attributes: attributes,
        provenance: TransportProvenance::unverified(),
        direct_response: None,
        reply_tx,
    }
}

/// A send as its kind log shows it. An unscoped I-Am-Router answer lists
/// its networks in the legacy table's hash order, so it is compared as a
/// set: sorted.
pub(super) fn sent(npdu: &[u8], kind: Kind) -> (Vec<u8>, Kind) {
    let npdu = if kind == Kind::Answer(0x01) {
        sorted_i_am(npdu)
    } else {
        npdu.to_vec()
    };
    (npdu, kind)
}

/// An I-Am-Router with no addresses, its networks sorted; anything else as
/// it is.
pub(super) fn sorted_i_am(npdu: &[u8]) -> Vec<u8> {
    match npdu {
        [1, 0x80, 0x01, list @ ..] if list.len() % 2 == 0 => {
            let mut networks: Vec<&[u8]> = list.chunks(2).collect();
            networks.sort_unstable();
            [&[1, 0x80, 0x01][..], &networks.concat()].concat()
        }
        _ => npdu.to_vec(),
    }
}

pub(super) fn legacy_kind(kind: legacy::SendKind) -> Kind {
    match kind {
        legacy::SendKind::Forward => Kind::Forward,
        legacy::SendKind::Global => Kind::Global,
        legacy::SendKind::Relay(code) => Kind::Relay(code),
        legacy::SendKind::Reject(reason) => Kind::Reject(reason),
        legacy::SendKind::Answer(code) => Kind::Answer(code),
        legacy::SendKind::Announce(code) => Kind::Announce(code),
        legacy::SendKind::Solicit => Kind::Solicit,
    }
}

/// What a port queue holds. I-Am-Router payloads are sorted, as in
/// [`sent`]; the kind log compares every other I-Am-Router exactly.
pub(super) fn drain_wire(rx: &mut mpsc::Receiver<SendRequest>) -> Vec<Wire> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|request| match request {
            SendRequest::Unicast {
                npdu,
                mac,
                data_attributes,
            } => Wire {
                to: Some(mac.to_vec()),
                npdu: sorted_i_am(&npdu),
                attributes: data_attributes,
            },
            SendRequest::Broadcast {
                npdu,
                data_attributes,
            } => Wire {
                to: None,
                npdu: sorted_i_am(&npdu),
                attributes: data_attributes,
            },
        })
        .collect()
}

pub(super) fn drain_queue<T, U: From<T>>(rx: &mut AdmissionReceiver<T>) -> Vec<U> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(U::from)
        .collect()
}

pub(super) fn admission(
    apdus: &AdmissionReceiver<ReceivedApdu>,
    controls: &AdmissionReceiver<ReceivedNetworkControl>,
) -> Counts {
    [apdus.counters().snapshot(), controls.counters().snapshot()]
        .into_iter()
        .flat_map(|snapshot| {
            [
                snapshot.current_depth as u64,
                snapshot.high_water as u64,
                snapshot.full_drops,
                snapshot.closed_drops,
                snapshot.fairness_drops,
            ]
        })
        .collect()
}

pub(super) fn decisions(gate: &ControlGate) -> Counts {
    let snapshot = gate.snapshot();
    [
        snapshot.i_am,
        snapshot.init_ack,
        snapshot.init_mgmt,
        snapshot.busy,
        snapshot.available,
        snapshot.reject,
        snapshot.i_could_be,
    ]
    .into_iter()
    .flat_map(|row| [row.allow_total, row.deny_total, row.policy_deny_total])
    .collect()
}
