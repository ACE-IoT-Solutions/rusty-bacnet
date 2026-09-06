//! BDT validation regression tests for issue #529 (owner security policy).
//!
//! Exercises `BbmdState::set_bdt` as the central commit choke point:
//! invalid or conflicting candidates must fail transactionally with an
//! `Error::Encoding`-style error and preserve the committed table.

use super::{BbmdState, BdtEntry};
use bacnet_types::error::Error;

const PORT: u16 = 0xBAC0;
const MASK_32: [u8; 4] = [255, 255, 255, 255];
const MASK_24: [u8; 4] = [255, 255, 255, 0];

fn make_bbmd() -> BbmdState {
    BbmdState::new([192, 168, 1, 1], PORT)
}

fn entry(ip: [u8; 4], port: u16, mask: [u8; 4]) -> BdtEntry {
    BdtEntry {
        ip,
        port,
        broadcast_mask: mask,
    }
}

fn err_text(err: Error) -> String {
    format!("{err}")
}

fn assert_encoding_error(err: Error, fragment: &str) -> String {
    assert!(
        matches!(err, Error::Encoding(_)),
        "expected Error::Encoding, got: {err:?}"
    );
    let text = err_text(err);
    assert!(
        text.to_lowercase().contains(fragment),
        "error text {text:?} must mention {fragment:?}"
    );
    text
}

#[test]
fn rejects_zero_port_and_preserves_table() {
    let mut bbmd = make_bbmd();
    let committed = entry([10, 0, 0, 5], PORT, MASK_32);
    bbmd.set_bdt(vec![committed.clone()]).unwrap();
    let before = bbmd.bdt().to_vec();

    let err = bbmd
        .set_bdt(vec![entry([10, 0, 0, 6], 0, MASK_32)])
        .unwrap_err();
    assert_encoding_error(err, "port");
    assert_eq!(bbmd.bdt(), before.as_slice());
}

#[test]
fn rejects_unspecified_ip() {
    let mut bbmd = make_bbmd();
    let err = bbmd
        .set_bdt(vec![entry([0, 0, 0, 0], PORT, MASK_32)])
        .unwrap_err();
    assert_encoding_error(err, "unspecified");
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_multicast_ip() {
    let mut bbmd = make_bbmd();
    for ip in [[224, 0, 0, 1], [239, 255, 255, 250]] {
        let err = bbmd.set_bdt(vec![entry(ip, PORT, MASK_32)]).unwrap_err();
        assert_encoding_error(err, "multicast");
        assert!(bbmd.bdt().is_empty(), "multicast {ip:?} must not commit");
    }
}

#[test]
fn rejects_limited_broadcast_ip() {
    let mut bbmd = make_bbmd();
    let err = bbmd
        .set_bdt(vec![entry([255, 255, 255, 255], PORT, MASK_32)])
        .unwrap_err();
    assert_encoding_error(err, "limited");
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_non_contiguous_mask() {
    let mut bbmd = make_bbmd();
    let err = bbmd
        .set_bdt(vec![entry([10, 0, 0, 5], PORT, [255, 0, 255, 0])])
        .unwrap_err();
    assert_encoding_error(err, "contiguous");
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_subnet_network_address() {
    let mut bbmd = make_bbmd();
    // 192.168.1.0/24 has all host bits zero.
    let err = bbmd
        .set_bdt(vec![entry([192, 168, 1, 0], PORT, MASK_24)])
        .unwrap_err();
    let text = assert_encoding_error(err, "network address");
    assert!(
        text.contains("192.168.1.0") || text.contains("network"),
        "{text:?}"
    );
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_subnet_broadcast_address() {
    let mut bbmd = make_bbmd();
    // 192.168.1.255/24 has all host bits one.
    let err = bbmd
        .set_bdt(vec![entry([192, 168, 1, 255], PORT, MASK_24)])
        .unwrap_err();
    assert_encoding_error(err, "broadcast");
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_slash_zero_derived_limited_broadcast() {
    let mut bbmd = make_bbmd();
    // /0 derives 255.255.255.255 for any peer IP and is unsafe.
    let err = bbmd
        .set_bdt(vec![entry([10, 0, 0, 5], PORT, [0, 0, 0, 0])])
        .unwrap_err();
    assert_encoding_error(err, "limited");
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn rejects_same_endpoint_different_mask_conflict_and_preserves_table() {
    let mut bbmd = make_bbmd();
    let committed = entry([10, 0, 0, 5], PORT, MASK_32);
    bbmd.set_bdt(vec![committed.clone()]).unwrap();
    let before = bbmd.bdt().to_vec();

    let conflicting = vec![
        entry([10, 0, 0, 9], PORT, MASK_32),
        entry([10, 0, 0, 9], PORT, MASK_24),
    ];
    let err = bbmd.set_bdt(conflicting).unwrap_err();
    assert_encoding_error(err, "conflict");
    assert_eq!(
        bbmd.bdt(),
        before.as_slice(),
        "conflicting candidate must preserve the committed table"
    );
}

#[test]
fn exact_duplicates_collapse_stably_without_duplicate_targets() {
    let mut bbmd = make_bbmd();
    let first = entry([10, 0, 0, 5], PORT, MASK_32);
    let second = entry([10, 0, 0, 6], PORT, MASK_32);
    bbmd.set_bdt(vec![
        first.clone(),
        second.clone(),
        first.clone(),
        second.clone(),
        first.clone(),
    ])
    .unwrap();
    // Stable first-seen order plus the auto-inserted self entry.
    assert_eq!(
        bbmd.bdt().len(),
        3,
        "duplicates must collapse: {0:?}",
        bbmd.bdt()
    );
    assert_eq!(bbmd.bdt()[0], first);
    assert_eq!(bbmd.bdt()[1], second);

    // Forwarding must not contain duplicate targets either.
    let targets = bbmd.forwarding_targets([192, 168, 1, 200], PORT);
    assert_eq!(targets.len(), 2);
    assert!(targets.contains(&([10, 0, 0, 5], PORT)));
    assert!(targets.contains(&([10, 0, 0, 6], PORT)));
}

#[test]
fn duplicates_reduce_before_capacity_accounting() {
    let mut bbmd = make_bbmd();
    // Build MAX distinct peers would need self; instead build MAX-1 distinct
    // peers and repeat them so the raw candidate exceeds MAX but the
    // canonicalized table (plus self) still fits.
    let mut distinct = Vec::new();
    for i in 0..(BbmdState::MAX_BDT_ENTRIES - 1) {
        // Avoid .0/.255 host endpoints and 0.0.0.0/multicast: use 10.x.y.z
        // with nonzero host octets. i < 127 here so third octet stays 0 and
        // last octet is 1..=127.
        let last = (i + 1) as u8;
        distinct.push(entry([10, 1, 0, last], PORT, MASK_32));
    }
    assert_eq!(distinct.len(), BbmdState::MAX_BDT_ENTRIES - 1);
    let mut raw = distinct.clone();
    raw.extend(distinct.iter().cloned());
    assert!(raw.len() > BbmdState::MAX_BDT_ENTRIES);
    bbmd.set_bdt(raw).unwrap();
    assert_eq!(bbmd.bdt().len(), BbmdState::MAX_BDT_ENTRIES);
}

#[test]
fn same_ip_different_port_remains_distinct() {
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(vec![
        entry([10, 0, 0, 5], PORT, MASK_32),
        entry([10, 0, 0, 5], PORT + 1, MASK_32),
    ])
    .unwrap();
    // Two distinct peers plus self.
    assert_eq!(bbmd.bdt().len(), 3);
    assert!(bbmd.is_bdt_peer([10, 0, 0, 5], PORT));
    assert!(bbmd.is_bdt_peer([10, 0, 0, 5], PORT + 1));
}

#[test]
fn allows_unicast_slash32_and_ordinary_slash24_peers() {
    let mut bbmd = make_bbmd();
    let unicast = entry([10, 0, 0, 5], PORT, MASK_32);
    let subnet_peer = entry([192, 168, 2, 10], PORT, MASK_24);
    let loopback_peer = entry([127, 0, 0, 2], PORT, MASK_32);
    let link_local = entry([169, 254, 10, 20], PORT, MASK_32);
    let documentation = entry([192, 0, 2, 44], PORT, MASK_24);
    bbmd.set_bdt(vec![
        unicast.clone(),
        subnet_peer.clone(),
        loopback_peer.clone(),
        link_local.clone(),
        documentation.clone(),
    ])
    .unwrap();
    for wanted in [
        &unicast,
        &subnet_peer,
        &loopback_peer,
        &link_local,
        &documentation,
    ] {
        assert!(
            bbmd.bdt().contains(wanted),
            "allowed peer missing: {wanted:?}"
        );
    }
}

#[test]
fn distinct_peers_sharing_forwarding_destination_are_not_deduplicated() {
    let mut bbmd = make_bbmd();
    // Both derive 192.168.7.255 via /24 but are distinct configured peers.
    let first = entry([192, 168, 7, 10], PORT, MASK_24);
    let second = entry([192, 168, 7, 20], PORT, MASK_24);
    bbmd.set_bdt(vec![first.clone(), second.clone()]).unwrap();
    assert!(bbmd.bdt().contains(&first));
    assert!(bbmd.bdt().contains(&second));
    assert_eq!(bbmd.bdt().len(), 3, "both peers plus self must be stored");
}

// Relocated from `bbmd.rs` inline tests to keep that file under the 700 LOC
// cap while preserving behavior. These peer-lookup tests use only valid
// entries and must keep passing under the new validation policy.
#[test]
fn relocated_is_bdt_peer_check() {
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(vec![entry([10, 0, 0, 1], PORT, MASK_32)])
        .unwrap();
    assert!(bbmd.is_bdt_peer([10, 0, 0, 1], PORT));
    assert!(!bbmd.is_bdt_peer([10, 0, 0, 2], PORT));
}

#[test]
fn relocated_forwarded_npdu_needs_local_broadcast_for_unicast_peer() {
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(vec![entry([10, 0, 0, 1], PORT, MASK_32)])
        .unwrap();
    assert!(bbmd.forwarded_npdu_needs_local_broadcast([10, 0, 0, 1], PORT));
}

#[test]
fn relocated_forwarded_npdu_skips_local_broadcast_for_directed_peer() {
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(vec![entry([10, 0, 0, 1], PORT, MASK_24)])
        .unwrap();
    assert!(!bbmd.forwarded_npdu_needs_local_broadcast([10, 0, 0, 1], PORT));
}

#[test]
fn relocated_forwarded_npdu_skips_local_broadcast_for_unknown_peer() {
    let bbmd = make_bbmd();
    assert!(!bbmd.forwarded_npdu_needs_local_broadcast([10, 0, 0, 1], PORT));
}
