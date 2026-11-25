"""Capture loopback B/IP datagrams and verify observed segmented exchanges."""

import asyncio
import hashlib
import time


def check(condition, message):
    if not condition:
        raise ValueError(message)


class Relay(asyncio.DatagramProtocol):
    """Forward original UDP payloads in both directions; decoding is offline."""

    def __init__(self):
        self.native = None
        self.java = ("127.0.0.1", 47809)
        self.frames = []
        self.changed = asyncio.Event()

    def connection_made(self, transport):
        self.transport = transport
        self.port = transport.get_extra_info("sockname")[1]

    def datagram_received(self, data, source):
        if source == self.java:
            dest = self.native
        elif source[0] == "127.0.0.1" and self.native in (None, source):
            self.native = source
            dest = self.java
        else:
            dest = None
        self.frames.append({"from": "java" if source == self.java else "native", "source": source,
                            "destination": dest, "time_ns": time.monotonic_ns(), "udp_hex": data.hex()})
        if dest is not None:
            self.transport.sendto(data, dest)
        self.changed.set()

    async def finish_read(self, start, service):
        # An SDK future may complete before this event loop forwards the final ACK.
        while True:
            self.changed.clear()
            entries = messages(self.frames[start:])
            final = next((p for side, p in entries if side == "java" and len(p) >= 5 and p[0] == 0x38
                          and p[4] == service), None)
            if final and any(side == "native" and p[:3] == bytes((0x40, final[1], final[2])) for side, p in entries):
                return
            await self.changed.wait()


def messages(frames):
    result = []
    for frame in frames:
        check(frame["destination"] is not None, "unrecognized sender on relay")
        data = bytes.fromhex(frame["udp_hex"])
        check(data[:2] == b"\x81\x0a" and int.from_bytes(data[2:4], "big") == len(data), "unexpected direct BVLC")
        check(data[4] == 1 and data[5] in (0, 4), "unexpected local NPDU")
        result.append((frame["from"], data[6:]))
    return result


def transfer(frames, service, *, confirmed=False):
    """Check sequence/window progress, with every ACK in its actual wire position."""
    entries = messages(frames)
    header = 6 if confirmed else 5
    data_type = 0 if confirmed else 3
    invoke_at = 2 if confirmed else 1
    first = next(p for side, p in entries if side == "java" and p[0] >> 4 == data_type and p[0] & 8)
    invoke = first[invoke_at]
    window = first[header - 2]
    check(window == 2, f"Java proposed window {window}")
    next_seq = 0
    owed = None
    final = False
    payload = bytearray()
    sizes, acks, order = [], [], []
    terminal = None
    request = None
    for side, apdu in entries:
        kind = apdu[0] >> 4
        if side == "native" and kind == 0 and not confirmed:
            check(request is None and apdu[2:4] == bytes((invoke, service)), "unexpected confirmed read request")
            request = apdu
        elif side == "java" and kind == data_type:
            check(confirmed or request is not None, "response arrived before the read request")
            check(not final and owed is None, "segment arrived before its required positive ACK")
            check(len(apdu) >= header and apdu[0] & 8 and apdu[invoke_at:header] ==
                  bytes((invoke, next_seq, window, service)), "segment invoke, service, sequence or window mismatch")
            sizes.append(len(apdu))
            payload.extend(apdu[header:])
            final = not apdu[0] & 4
            order.append(f"segment{next_seq}")
            if next_seq % window == 0 or final:
                owed = next_seq
            next_seq += 1
        elif side == "native" and kind == 4:
            check(owed is not None and apdu == bytes((0x41 if confirmed else 0x40, invoke, owed, window)),
                  f"unexpected SegmentACK {apdu.hex()}")
            acks.append(owed)
            order.append(f"ack{owed}")
            owed = None
        elif side == "native" and confirmed and kind in (2, 5):
            check(final and owed is None and terminal is None and apdu[1:3] == bytes((invoke, service)),
                  "terminal COV result arrived before final SegmentACK or with wrong identity")
            terminal = apdu.hex()
        else:
            raise ValueError(f"unexpected APDU during service {service}: {side} {apdu[:8].hex()}")
    check(next_seq > 2 and final and owed is None, "incomplete segmented exchange")
    receipt = {"service": service, "invoke": invoke, "window": window, "segments": next_seq,
               "positive_ack_sequences": acks, "wire_order": order, "apdu_lengths": sizes,
               "max_segment_apdu": max(sizes), "payload_sha256": hashlib.sha256(payload).hexdigest(),
               "terminal_apdu_hex": terminal}
    if confirmed:
        check(terminal is not None, "missing confirmed COV result")
    else:
        check(request is not None and request[0] & 2, "read did not advertise segmented responses")
        receipt["request_apdu_hex"] = request.hex()
        receipt["advertised_max_apdu"] = (50, 128, 206, 480, 1024, 1476)[request[1] & 15]
    return receipt, bytes(payload)
