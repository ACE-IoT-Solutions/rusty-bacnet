//! pcap export: the classic libpcap file format, one file per link type.
//!
//! - B/IP as Ethernet (LINKTYPE 1) or raw IPv4 (LINKTYPE 228). The tap
//!   hands over the BVLL message only, the UDP payload, so the IPv4 and UDP
//!   headers (and for Ethernet a frame header) are made up from the
//!   endpoints the frame was captured with. The UDP checksum is zero, which
//!   IPv4 allows to mean none; the IPv4 header checksum is computed.
//! - MS/TP as LINKTYPE 165, each frame from its preamble.
//!
//! B/IPv6, BACnet/SC and the monitor's line errors are left out: B/IPv6
//! would need IPv6 headers, and SC needs pcapng with its own encapsulation
//! (a follow-up).

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CaptureFrame, ExportJob, Framing};

/// LINKTYPE_ETHERNET.
pub(crate) const LINKTYPE_ETHERNET: u32 = 1;
/// LINKTYPE_BACNET_MS_TP.
pub(crate) const LINKTYPE_MSTP: u32 = 165;
/// LINKTYPE_IPV4.
pub(crate) const LINKTYPE_IPV4: u32 = 228;

const SNAPLEN: u32 = 65_535;

/// The file header: microsecond timestamps, version 2.4, little-endian.
fn file_header(link_type: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.extend_from_slice(&0xA1B2_C3D4_u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&SNAPLEN.to_le_bytes());
    out.extend_from_slice(&link_type.to_le_bytes());
    out
}

fn record(out: &mut Vec<u8>, time: SystemTime, packet: &[u8]) {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = u32::try_from(since.as_secs()).unwrap_or(u32::MAX);
    let length = u32::try_from(packet.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&seconds.to_le_bytes());
    out.extend_from_slice(&since.subsec_micros().to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(packet);
}

/// The one's-complement sum of a header, as IPv4 checksums it.
fn checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = header
        .chunks(2)
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])))
        .sum();
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn v4(address: SocketAddr) -> Option<(Ipv4Addr, u16)> {
    match address.ip() {
        IpAddr::V4(ip) => Some((ip, address.port())),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(|ip| (ip, address.port())),
    }
}

/// An IPv4 packet with a UDP datagram holding `payload`; `None` without
/// IPv4 endpoints or for a payload too long for one datagram.
pub(crate) fn ipv4_udp(
    source: SocketAddr,
    destination: SocketAddr,
    id: u16,
    payload: &[u8],
) -> Option<Vec<u8>> {
    let ((src_ip, src_port), (dst_ip, dst_port)) = (v4(source)?, v4(destination)?);
    let udp_length = u16::try_from(8 + payload.len()).ok()?;
    let total = u16::try_from(20 + 8 + payload.len()).ok()?;
    let mut packet = Vec::with_capacity(usize::from(total));
    packet.extend_from_slice(&[0x45, 0]);
    packet.extend_from_slice(&total.to_be_bytes());
    packet.extend_from_slice(&id.to_be_bytes());
    // Don't fragment; time to live 64; UDP.
    packet.extend_from_slice(&[0x40, 0, 64, 17, 0, 0]);
    packet.extend_from_slice(&src_ip.octets());
    packet.extend_from_slice(&dst_ip.octets());
    let sum = checksum(&packet);
    packet[10..12].copy_from_slice(&sum.to_be_bytes());
    packet.extend_from_slice(&src_port.to_be_bytes());
    packet.extend_from_slice(&dst_port.to_be_bytes());
    packet.extend_from_slice(&udp_length.to_be_bytes());
    packet.extend_from_slice(&[0, 0]);
    packet.extend_from_slice(payload);
    Some(packet)
}

/// A made-up Ethernet MAC for an IPv4 address: locally administered,
/// carrying the address, or broadcast for a broadcast.
fn ethernet_mac(ip: Ipv4Addr) -> [u8; 6] {
    if ip.is_broadcast() || ip.octets()[3] == 255 {
        return [0xFF; 6];
    }
    let [a, b, c, d] = ip.octets();
    [0x02, 0x00, a, b, c, d]
}

/// The B/IP frames of `frames` as a pcap file; `None` when there are none
/// with endpoints to put in the made-up headers.
pub(crate) fn bip_file(frames: &[CaptureFrame], raw_ipv4: bool) -> Option<Vec<u8>> {
    let link_type = if raw_ipv4 {
        LINKTYPE_IPV4
    } else {
        LINKTYPE_ETHERNET
    };
    let mut out = file_header(link_type);
    let mut written = 0u16;
    for frame in frames.iter().filter(|f| f.framing == Framing::Bip) {
        let Some((source, destination)) = frame.udp else {
            continue;
        };
        let Some(packet) = ipv4_udp(source, destination, written, &frame.bytes) else {
            continue;
        };
        if raw_ipv4 {
            record(&mut out, frame.time, &packet);
        } else {
            let (src, dst) = (v4(source)?.0, v4(destination)?.0);
            let mut ethernet = Vec::with_capacity(14 + packet.len());
            ethernet.extend_from_slice(&ethernet_mac(dst));
            ethernet.extend_from_slice(&ethernet_mac(src));
            ethernet.extend_from_slice(&[0x08, 0x00]);
            ethernet.extend_from_slice(&packet);
            record(&mut out, frame.time, &ethernet);
        }
        written = written.wrapping_add(1);
    }
    (written > 0).then_some(out)
}

/// The MS/TP frames of `frames` as a pcap file; `None` when there are none.
pub(crate) fn mstp_file(frames: &[CaptureFrame]) -> Option<Vec<u8>> {
    let mut out = file_header(LINKTYPE_MSTP);
    let mut any = false;
    for frame in frames {
        if frame.framing == Framing::Mstp && !frame.bytes.is_empty() {
            record(&mut out, frame.time, &frame.bytes);
            any = true;
        }
    }
    any.then_some(out)
}

/// Write `job` into `dir`: `STEM-bip.pcap` and `STEM-mstp.pcap`, for the
/// links it has frames of. Returns the paths written. Blocking.
pub(crate) fn export(job: &ExportJob, dir: &Path) -> Result<Vec<String>, String> {
    let files = [
        ("bip", bip_file(&job.frames, job.raw_ipv4)),
        ("mstp", mstp_file(&job.frames)),
    ];
    let mut written = Vec::new();
    for (link, contents) in files {
        let Some(contents) = contents else {
            continue;
        };
        let path: PathBuf = dir.join(format!("{}-{link}.pcap", job.stem));
        write_new(&path, &contents).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path.display().to_string());
    }
    if written.is_empty() {
        return Err("nothing to export: no B/IP or MS/TP frames in the ring".into());
    }
    Ok(written)
}

/// Create `path`, refusing to replace a file that is there.
fn write_new(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}
