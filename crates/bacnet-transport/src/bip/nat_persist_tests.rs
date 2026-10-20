//! A NAT setting activated through the Network Port is saved with the BDT,
//! and a restart loads the two together, over the configured ones (#940).

use super::bdt_activation_persist_tests::{row, saved, tables, TempDir};
use super::*;
use crate::bbmd::NatTraversal;
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::BACnetHostNPort;
use bytes::BytesMut;
use std::fs;
use std::path::Path;

const GLOBAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 1), 47809);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 2), 47809);

fn two_hop(address: SocketAddrV4) -> BdtEntry {
    BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: [255; 4],
    }
}

/// A started BBMD on loopback with `path` as its persisted-BDT file, behind
/// NAT at [`GLOBAL`] when `nat`, with `bdt` configured.
async fn bbmd(path: &Path, nat: bool, bdt: Vec<BdtEntry>) -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(bdt);
    transport.set_bdt_persist_path(path.to_path_buf());
    transport.set_bbmd_nat_traversal(nat.then(|| NatTraversal::new(GLOBAL)));
    let _rx = transport.start().await.unwrap();
    transport
}

/// The started BBMD's NAT setting, own address and BDT.
fn running(transport: &BipTransport) -> (bool, Option<SocketAddrV4>, SocketAddrV4, Vec<BdtEntry>) {
    let state = bbmd::lock(transport.bbmd_state().unwrap());
    (
        state.nat_traversal(),
        state.global_address(),
        state.own_address(),
        state.bdt(),
    )
}

#[tokio::test]
async fn nat_turned_on_over_the_network_survives_a_restart() {
    let dir = TempDir::new("nat-on");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(true);
    changes.global_address = Some(BACnetHostNPort::from_socket_addr(GLOBAL.into()));
    changes.broadcast_distribution_table = Some(vec![
        row(GLOBAL.ip().octets(), GLOBAL.port()),
        row(PEER.ip().octets(), PEER.port()),
    ]);
    tables(&transport).activate(&changes).unwrap();
    saved(&transport, 1).await;
    transport.stop().await.unwrap();

    // The configuration still says no NAT; the activated setting wins.
    let mut restarted = bbmd(&path, false, Vec::new()).await;
    assert_eq!(
        running(&restarted),
        (
            true,
            Some(GLOBAL),
            GLOBAL,
            vec![two_hop(GLOBAL), two_hop(PEER)]
        )
    );
    restarted.stop().await.unwrap();
}

#[tokio::test]
async fn nat_turned_off_over_the_network_survives_a_restart() {
    let dir = TempDir::new("nat-off");
    let path = dir.file("bdt.bin");
    let config = vec![two_hop(GLOBAL), two_hop(PEER)];
    let mut transport = bbmd(&path, true, config.clone()).await;
    assert!(running(&transport).0);
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(false);
    tables(&transport).activate(&changes).unwrap();
    saved(&transport, 1).await;
    let local = transport.bip_port().unwrap().endpoint;
    transport.stop().await.unwrap();

    // The configuration still says NAT; the activated setting wins, and the
    // old global row went with it.
    let mut restarted = bbmd(&path, true, config).await;
    let (nat, global, own, rows) = running(&restarted);
    assert!(!nat);
    assert_eq!(global, Some(GLOBAL), "the address is kept for later");
    assert_eq!(*own.ip(), Ipv4Addr::LOCALHOST);
    assert!(!rows.contains(&two_hop(GLOBAL)), "{rows:?}");
    assert!(rows.contains(&two_hop(PEER)));
    assert_eq!(local.ip(), own.ip());
    restarted.stop().await.unwrap();
}

/// A file from before #940, rows only, loads as it always did, and the
/// configured NAT setting applies.
#[tokio::test]
async fn a_file_without_the_trailer_keeps_the_configured_nat() {
    let dir = TempDir::new("nat-old-file");
    let path = dir.file("bdt.bin");
    let mut rows = BytesMut::new();
    bbmd::encode_bdt_entries(&[two_hop(GLOBAL), two_hop(PEER)], &mut rows);
    fs::write(&path, &rows).unwrap();
    let mut transport = bbmd(&path, true, Vec::new()).await;
    assert_eq!(
        running(&transport),
        (
            true,
            Some(GLOBAL),
            GLOBAL,
            vec![two_hop(GLOBAL), two_hop(PEER)]
        )
    );
    transport.stop().await.unwrap();
}

/// A saved set that fails validation (NAT on, but the rows lack the
/// global address) is refused, and the configured values apply.
#[tokio::test]
async fn a_persisted_set_that_breaks_the_nat_rules_falls_back_to_the_config() {
    let dir = TempDir::new("nat-bad-file");
    let path = dir.file("bdt.bin");
    let mut octets = BytesMut::new();
    bbmd::encode_bdt_entries(&[two_hop(PEER)], &mut octets);
    octets.extend_from_slice(b"NAT1\x01");
    octets.extend_from_slice(&GLOBAL.ip().octets());
    octets.extend_from_slice(&GLOBAL.port().to_be_bytes());
    fs::write(&path, &octets).unwrap();
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let (nat, global, own, rows) = running(&transport);
    assert!(!nat);
    assert_eq!(global, None);
    assert_eq!(rows, [two_hop(own)], "the configured, empty BDT");
    transport.stop().await.unwrap();
}
