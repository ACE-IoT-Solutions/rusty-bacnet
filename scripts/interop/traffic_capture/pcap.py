"""Audit classic Ethernet pcap from Linux lo. A record has exactly one disposition."""
from dataclasses import dataclass
import hashlib
import socket
import struct

from oracle import IP, scenario


@dataclass
class Record:
    timestamp_ns: int
    data: bytes


def records(raw, partial=False):
    if len(raw) < 24 and partial:
        return []
    formats = {bytes.fromhex(m): (e, scale) for m, e, scale in (
        ("d4c3b2a1", "<", 1000), ("a1b2c3d4", ">", 1000),
        ("4d3cb2a1", "<", 1), ("a1b23c4d", ">", 1))}
    if raw[:4] not in formats or len(raw) < 24:
        raise ValueError("invalid or truncated classic pcap header")
    order, scale = formats[raw[:4]]
    major, minor, _, _, snap, link = struct.unpack_from(order + "HHIIII", raw, 4)
    if (major, minor, link) != (2, 4, 1) or not snap:
        raise ValueError(f"unexpected pcap version, link type or snaplen: {major}.{minor}/{link}/{snap}")
    result = []
    offset = 24
    while offset < len(raw):
        if len(raw) - offset < 16:
            if partial:
                break
            raise ValueError("truncated record header")
        sec, fraction, captured, original = struct.unpack_from(order + "IIII", raw, offset)
        if captured != original or not 0 < captured <= snap or fraction * scale >= 10**9:
            raise ValueError(f"incomplete or invalid record {len(result) + 1}")
        offset += 16
        if offset + captured > len(raw):
            if partial:
                break
            raise ValueError("truncated record body")
        result.append(Record(sec * 10**9 + fraction * scale, raw[offset:offset + captured]))
        offset += captured
    return result


def udp(data):
    if len(data) < 42 or data[12:14] != b"\x08\x00":
        raise ValueError("unclassified Ethernet frame (expected IPv4)")
    ip = data[14:]
    if ip[0] != 0x45 or ip[9] != 17 or int.from_bytes(ip[6:8], "big") & 0xbfff:
        raise ValueError("unclassified IPv4 packet (expected unfragmented UDP, no options)")
    if len(ip) != int.from_bytes(ip[2:4], "big"):
        raise ValueError("incorrect IPv4 packet length")
    if ip[12:20] != socket.inet_aton(IP) * 2:
        raise ValueError("unexpected IPv4 source or destination")
    src, dst, size = struct.unpack_from(">HHH", ip, 20)
    if size != len(ip) - 20:
        raise ValueError("incorrect UDP length")
    return src, dst, ip[28:]


def audit(raw, owner, nonce, idle_seconds):
    packets = records(raw)
    expected = scenario(owner, nonce)
    invokes, accepted = {}, []
    for index, record in enumerate(packets):
        try:
            src, dst, payload = udp(record.data)
            if index >= len(expected):
                raise ValueError("unexpected datagram after final marker")
            step = expected[index]
            step.match(src, dst, payload, invokes)
            accepted.append(dict(record=index + 1, action=step.name, src_port=src, dst_port=dst,
                                 udp_bytes=len(payload), sha256=hashlib.sha256(payload).hexdigest(),
                                 timestamp_ns=record.timestamp_ns))
        except (ValueError, KeyError) as error:
            raise ValueError(f"record {index + 1} at {record.timestamp_ns}, frame={record.data.hex()}: {error}") from error
    if len(accepted) != len(expected):
        raise ValueError(f"capture ended before {expected[len(accepted)].name}")
    duration = (packets[2].timestamp_ns - packets[1].timestamp_ns) / 1e9
    if duration < idle_seconds:
        raise ValueError(f"captured idle interval {duration} shorter than requested {idle_seconds}")
    return dict(accepted=accepted, invokes=invokes, linktype="EN10MB", idle_seconds=duration,
                total_seconds=(packets[-1].timestamp_ns - packets[0].timestamp_ns) / 1e9)
