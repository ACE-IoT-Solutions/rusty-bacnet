//! pcap export (#971): the file and record headers, the IPv4 and UDP
//! headers made up for B/IP frames, MS/TP from the preamble, the files the
//! coordinator writes, and, where `tshark` is on PATH, Wireshark's reading
//! of them.

use std::path::Path;

use bacnet_transport::mstp::codec::FrameType;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;
use tokio::sync::mpsc;

use super::capture::{bip_frame, mixed, wall};
use super::site::bounded;
use super::*;
use crate::decode::frame::summarize_frame;
use crate::decode::testing;
use crate::tui::capture::pcap::{
    bip_file, export, ipv4_udp, mstp_file, LINKTYPE_ETHERNET, LINKTYPE_IPV4, LINKTYPE_MSTP,
};
use crate::tui::capture::{CaptureEvent, CaptureFrame, Direction, ExportJob, Framing};
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::{coordinator, EventSink, EVENT_CHANNEL_CAPACITY};

fn u32_at(file: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(file[at..at + 4].try_into().unwrap())
}

/// The records of a pcap file: (seconds, microseconds, packet).
fn records(file: &[u8]) -> Vec<(u32, u32, &[u8])> {
    let mut out = Vec::new();
    let mut at = 24;
    while at < file.len() {
        let length = u32_at(file, at + 8) as usize;
        assert_eq!(u32_at(file, at + 12) as usize, length, "captured whole");
        out.push((
            u32_at(file, at),
            u32_at(file, at + 4),
            &file[at + 16..at + 16 + length],
        ));
        at += 16 + length;
    }
    out
}

/// The one's-complement sum over a header that holds its checksum: zero
/// when the checksum is right.
fn ones_sum(header: &[u8]) -> u16 {
    let mut sum: u32 = header
        .chunks(2)
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], pair[1]])))
        .sum();
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn mstp_frames() -> Vec<CaptureFrame> {
    let rp = testing::npdu(&testing::rp_request(4), true);
    [
        testing::mstp(FrameType::TOKEN, 2, 1, &[]),
        testing::mstp(FrameType::POLL_FOR_MASTER, 3, 2, &[]),
        testing::mstp(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 2, &rp),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, bytes)| CaptureFrame {
        time: wall(i as u64 * 10),
        framing: Framing::Mstp,
        direction: Direction::Wire,
        udp: None,
        source: String::new(),
        destination: String::new(),
        via: None,
        summary: summarize_frame(Framing::Mstp, &bytes),
        bytes: Bytes::from(bytes),
    })
    .collect()
}

#[test]
fn bip_frames_get_ethernet_ipv4_and_udp_headers() {
    let frames = mixed();
    let file = bip_file(&frames, false).unwrap();
    assert_eq!(u32_at(&file, 0), 0xA1B2_C3D4);
    assert_eq!(&file[4..8], &[2, 0, 4, 0], "version 2.4");
    assert_eq!(u32_at(&file, 20), LINKTYPE_ETHERNET);
    let records = records(&file);
    assert_eq!(records.len(), frames.len());
    let (seconds, micros, packet) = records[1];
    assert_eq!((seconds, micros), (1_760_000_000, 12_000));
    assert_eq!(&packet[12..14], &[0x08, 0x00], "IPv4");
    assert_eq!(&packet[0..6], &[0x02, 0x00, 10, 0, 0, 1], "to 10.0.0.1");
    let ip = &packet[14..];
    assert_eq!(ip[0], 0x45);
    assert_eq!(ip[9], 17, "UDP");
    assert_eq!(ones_sum(&ip[..20]), 0, "header checksum");
    assert_eq!(&ip[12..16], &[10, 0, 0, 2]);
    assert_eq!(&ip[16..20], &[10, 0, 0, 1]);
    let udp = &ip[20..];
    assert_eq!(u16::from_be_bytes([udp[0], udp[1]]), 47808);
    assert_eq!(
        usize::from(u16::from_be_bytes([udp[4], udp[5]])),
        8 + frames[1].bytes.len()
    );
    assert_eq!(&udp[8..], &frames[1].bytes[..]);

    // Raw IPv4 starts at the IP header.
    let file = bip_file(&frames, true).unwrap();
    assert_eq!(u32_at(&file, 20), LINKTYPE_IPV4);
    let records = self::records(&file);
    assert_eq!(records[0].2[0], 0x45);
    assert_eq!(&records[0].2[28..], &frames[0].bytes[..]);

    // A broadcast goes to the Ethernet broadcast address.
    let broadcast = ipv4_udp(
        "10.0.0.1:47808".parse().unwrap(),
        "10.0.0.255:47808".parse().unwrap(),
        0,
        &[0x81, 0x0B, 0x00, 0x04],
    )
    .unwrap();
    assert_eq!(broadcast.len(), 32);
    // Without endpoints, or without B/IP frames, there is no file.
    assert!(bip_file(&mstp_frames(), false).is_none());
}

#[test]
fn mstp_frames_are_written_from_their_preamble() {
    let frames = mstp_frames();
    let file = mstp_file(&frames).unwrap();
    assert_eq!(u32_at(&file, 20), LINKTYPE_MSTP);
    let records = records(&file);
    assert_eq!(records.len(), 3);
    for ((_, _, packet), frame) in records.iter().zip(&frames) {
        assert_eq!(*packet, &frame.bytes[..]);
        assert_eq!(&packet[..2], &[0x55, 0xFF]);
    }
    // A line error has no octets to write.
    let mut errors = frames.clone();
    for frame in &mut errors {
        frame.bytes = Bytes::new();
    }
    assert!(mstp_file(&errors).is_none());
}

#[test]
fn export_writes_one_file_per_link_and_never_replaces_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut frames = mixed();
    frames.extend(mstp_frames());
    let job = ExportJob {
        frames,
        raw_ipv4: false,
        stem: "capture-test".into(),
    };
    let written = export(&job, dir.path()).unwrap();
    assert_eq!(written.len(), 2);
    assert!(written[0].ends_with("capture-test-bip.pcap"));
    assert!(written[1].ends_with("capture-test-mstp.pcap"));
    let error = export(&job, dir.path()).unwrap_err();
    assert!(error.contains("capture-test-bip.pcap"), "{error}");
    let empty = ExportJob {
        frames: Vec::new(),
        ..job
    };
    assert!(export(&empty, dir.path())
        .unwrap_err()
        .contains("nothing to export"));
}

/// `e` reaches the coordinator, which writes into the profile workspace
/// on a blocking thread and reports the files.
#[tokio::test]
async fn the_coordinator_writes_the_export_and_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = SessionOptions::default();
    options.watch.workspace = Some(dir.path().to_path_buf());
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(None, None, sink, rx, options);
    let mut app = super::capture::capturing();
    super::capture::deliver(&mut app, mixed(), 0);
    for command in press(&mut app, KeyCode::Char('e')) {
        commands.send(command).await.unwrap();
    }
    let reply = bounded(events.recv()).await.unwrap();
    let WorkerEvent::Capture(CaptureEvent::Exported(Ok(files))) = &reply else {
        panic!("{reply:?}");
    };
    assert_eq!(files.len(), 1);
    assert!(Path::new(&files[0]).starts_with(dir.path()));
    let file = std::fs::read(&files[0]).unwrap();
    assert_eq!(records(&file).len(), 7);
    worker(&mut app, reply);
    assert!(app.flash.as_ref().unwrap().text.starts_with("Exported "));
    drop(commands);
    bounded(task).await.unwrap();
}

/// `tshark -r` reads every exported file as BVLC, BACnet and BACnet-APDU,
/// or MS/TP, with no malformed packet. Skipped where tshark isn't on PATH.
#[test]
fn tshark_reads_the_exports_without_malformed_packets() {
    let Ok(version) = std::process::Command::new("tshark")
        .arg("--version")
        .output()
    else {
        eprintln!("tshark is not on PATH: skipped");
        return;
    };
    assert!(version.status.success());
    let dir = tempfile::tempdir().unwrap();
    let mut bip = mixed();
    let ack = bip_frame(1_500, false, &testing::rp_ack(1), false);
    let error = testing::error(5, ConfirmedServiceChoice::WRITE_PROPERTY);
    bip.extend([ack, bip_frame(1_600, false, &error, false)]);
    let cases = [
        (
            bip_file(&bip, false).unwrap(),
            "eth:",
            &["bvlc", "bacnet", "bacapp"][..],
        ),
        (
            bip_file(&bip, true).unwrap(),
            "ip:",
            &["bvlc", "bacnet", "bacapp"][..],
        ),
        (
            mstp_file(&mstp_frames()).unwrap(),
            "mstp",
            &["mstp", "bacnet", "bacapp"][..],
        ),
    ];
    for (i, (file, first, protocols)) in cases.iter().enumerate() {
        let path = dir.path().join(format!("{i}.pcap"));
        std::fs::write(&path, file).unwrap();
        let fields = tshark(&path, &["-T", "fields", "-e", "frame.protocols"]);
        assert!(fields.lines().all(|l| l.starts_with(first)), "{fields}");
        for protocol in *protocols {
            assert!(fields.contains(protocol), "{protocol} in {fields}");
        }
        let malformed = tshark(
            &path,
            &["-Y", "_ws.malformed || _ws.expert.severity >= error"],
        );
        assert!(malformed.trim().is_empty(), "malformed: {malformed}");
    }
}

fn tshark(path: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("tshark")
        .arg("-r")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
