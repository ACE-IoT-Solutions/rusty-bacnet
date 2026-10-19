//! A BDT activated through the Network Port object is saved to the
//! persisted-BDT file, which a restarted transport loads; a failed save is
//! counted and leaves the activation in place (#1826).

use super::*;
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostNPort};
use bytes::BytesMut;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A directory of its own under the system's temporary directory, removed
/// with everything in it when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusty-bacnet-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn row(ip: [u8; 4], port: u16) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort::from_socket_addr(SocketAddrV4::new(ip.into(), port).into()),
        broadcast_mask: Some([0xFF; 4]),
    }
}

/// A started BBMD on loopback with an empty configured BDT and `path` as
/// its persisted-BDT file.
async fn bbmd_at(path: &Path) -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(Vec::new());
    transport.set_bdt_persist_path(path.to_path_buf());
    let _rx = transport.start().await.unwrap();
    transport
}

fn tables(transport: &BipTransport) -> Arc<dyn BbmdTables> {
    match transport.bip_port().unwrap().mode {
        BipPortMode::Bbmd {
            tables: Some(tables),
        } => tables,
        other => panic!("{other:?}"),
    }
}

fn bdt_changes(table: Vec<BACnetBDTEntry>) -> BbmdChanges {
    let mut changes = BbmdChanges::default();
    changes.broadcast_distribution_table = Some(table);
    changes
}

#[tokio::test]
async fn an_activated_bdt_is_saved_and_a_restart_loads_it() {
    let dir = TempDir::new("bdt-activation");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    assert!(
        !path.exists(),
        "nothing is written until a BDT is activated"
    );
    let own = transport.bip_port().unwrap().endpoint;
    let table = vec![
        row([10, 0, 0, 2], 0xBAC0),
        row(own.ip().octets(), own.port()),
    ];

    // A change that leaves the BDT alone doesn't touch the file.
    let mut accept = BbmdChanges::default();
    accept.accept_foreign_device_registrations = Some(true);
    tables(&transport).activate(&accept).unwrap();
    assert!(!path.exists());

    tables(&transport)
        .activate(&bdt_changes(table.clone()))
        .unwrap();
    assert_eq!(tables(&transport).broadcast_distribution_table(), table);
    let mut expected = BytesMut::new();
    let rows = [
        bbmd::BdtEntry {
            ip: [10, 0, 0, 2],
            port: 0xBAC0,
            broadcast_mask: [0xFF; 4],
        },
        bbmd::BdtEntry {
            ip: own.ip().octets(),
            port: own.port(),
            broadcast_mask: [0xFF; 4],
        },
    ];
    bbmd::encode_bdt_entries(&rows, &mut expected);
    assert_eq!(fs::read(&path).unwrap(), expected.to_vec());
    assert!(
        !dir.file("bdt.bin.tmp").exists(),
        "the temporary file is renamed"
    );
    assert_eq!(transport.bdt_persist_failures(), 0);
    transport.stop().await.unwrap();

    // A new transport with the same file starts from the activated BDT, not
    // its configured (empty) one.
    let mut restarted = BipTransport::new(Ipv4Addr::LOCALHOST, own.port(), Ipv4Addr::BROADCAST);
    restarted.enable_bbmd(Vec::new());
    restarted.set_bdt_persist_path(path.clone());
    let _rx = restarted.start().await.unwrap();
    assert_eq!(tables(&restarted).broadcast_distribution_table(), table);
    restarted.stop().await.unwrap();
}

#[tokio::test]
async fn a_failed_save_is_counted_and_keeps_the_activation() {
    let dir = TempDir::new("bdt-activation-failure");
    // The file's directory doesn't exist, so neither the temporary file nor
    // the rename can be made.
    let path = dir.file("missing").join("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    let table = vec![row([10, 0, 0, 3], 0xBAC0)];
    tables(&transport).activate(&bdt_changes(table)).unwrap();
    assert_eq!(transport.bdt_persist_failures(), 1);
    assert!(!path.exists());
    let in_use = tables(&transport).broadcast_distribution_table();
    assert!(in_use.contains(&row([10, 0, 0, 3], 0xBAC0)), "{in_use:?}");
    transport.stop().await.unwrap();
}

/// A save replaces the file whole: a shorter BDT leaves no rows of the
/// longer one behind.
#[tokio::test]
async fn a_shorter_bdt_replaces_the_file_whole() {
    let dir = TempDir::new("bdt-activation-shrink");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    let view = tables(&transport);
    view.activate(&bdt_changes(vec![
        row([10, 0, 0, 2], 0xBAC0),
        row([10, 0, 0, 3], 0xBAC0),
    ]))
    .unwrap();
    let long = fs::read(&path).unwrap().len();
    view.activate(&bdt_changes(vec![row([10, 0, 0, 4], 0xBAC0)]))
        .unwrap();
    let short = fs::read(&path).unwrap();
    assert!(short.len() < long);
    let saved = BbmdState::decode_bdt(&short).unwrap();
    assert!(saved.iter().any(|entry| entry.ip == [10, 0, 0, 4]));
    assert!(!saved.iter().any(|entry| entry.ip == [10, 0, 0, 2]));
    transport.stop().await.unwrap();
}
