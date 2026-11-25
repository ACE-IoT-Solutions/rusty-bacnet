"""Scripted B/IP router for endpoint_workload.py. Python standard library only."""

from collections import Counter
import hashlib
import json
import socket
import struct
import sys
import time

DNET = 5
STATION = b"\x0a"
PROCESS = 7
DEVICE_INSTANCE = 2589
RP, RPM, COV = 12, 14, 1
NAMES = {RP: "rp", RPM: "rpm", COV: "cov"}
CHUNK = 480
WINDOW = 2
TEXT_BYTES = 1440
# Analog Input 1 Description, or Device 10 Description, Object_List[0] and Present_Value.
REQUESTS = {
    RP: bytes.fromhex("0c 00000001 19 1c"),
    RPM: bytes.fromhex("0c 0200000a 1e 09 1c 09 4c 19 00 09 55 1f"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def text_value(seed, number, name):
    identity = f"{name}/{number}:"
    digest = hashlib.sha256(f"{seed}/{identity}".encode()).hexdigest()
    return (identity + digest * (1 + TEXT_BYTES // len(digest)))[:TEXT_BYTES]


def string_tag(text):
    encoded = b"\x00" + text.encode("ascii")
    return b"\x75\xfe" + len(encoded).to_bytes(2, "big") + encoded


def service_data(service, seed, number):
    value = string_tag(text_value(seed, number, NAMES[service]))
    if service == RP:
        return REQUESTS[RP] + b"\x3e" + value + b"\x3f"
    if service == RPM:
        return (bytes.fromhex("0c0200000a 1e 291c 4e") + value
                + bytes.fromhex("4f 294c 3900 4e 21") + bytes([7 + number % 2])
                + bytes.fromhex("4f 2955 5e 9102 9120 5f 1f"))
    return (b"\x09" + bytes([PROCESS]) + b"\x1c" + ((8 << 22) | DEVICE_INSTANCE).to_bytes(4, "big")
            + bytes.fromhex("2c00000001 393c 4e 091c 2e") + value + bytes.fromhex("2f 4f"))


def emit(content):
    print(json.dumps(content, separators=(",", ":")), flush=True)


class Router:
    def __init__(self, link):
        self.link = link
        self.counts = Counter()
        self.acks = {}

    def receive(self):
        data = self.link.recv(65535)
        self.counts["rx_packets"] += 1
        self.counts["rx_octets"] += len(data)
        require(data[:2] == b"\x81\x0a" and int.from_bytes(data[2:4], "big") == len(data),
                "unexpected BVLC header")
        # Routed destination, optionally expecting a reply, no source specifier.
        require(data[4:6] in (b"\x01\x20", b"\x01\x24")
                and data[6:10] == DNET.to_bytes(2, "big") + b"\x01" + STATION,
                "unexpected NPDU destination or source")
        apdu = data[11:]  # hop count precedes the APDU
        if apdu and apdu[0] >> 4 in (5, 6, 7):
            self.counts["rx_error_reject_abort"] += 1
        return apdu

    def send(self, apdu):
        npdu = b"\x01\x08" + DNET.to_bytes(2, "big") + b"\x01" + STATION + apdu
        frame = b"\x81\x0a" + struct.pack(">H", len(npdu) + 4) + npdu
        sent_at = time.monotonic_ns()
        self.link.send(frame)
        self.counts["tx_packets"] += 1
        self.counts["tx_octets"] += len(frame)
        return sent_at

    def request(self, remaining):
        apdu = self.receive()
        require(len(apdu) >= 4 and apdu[0] == 2, "expected confirmed read accepting segmented response")
        service = apdu[3]
        require(service in remaining and apdu[4:] == REQUESTS[service], "unexpected read service or selectors")
        self.counts[f"rx_request_{NAMES[service]}"] += 1
        return service, apdu[2]

    def transfer(self, service, invoke, body):
        name = NAMES[service]
        chunks = [body[start:start + CHUNK] for start in range(0, len(body), CHUNK)]
        self.acks[name] = []
        first_send = None
        for seq, chunk in enumerate(chunks):
            last = seq + 1 == len(chunks)
            more = 0 if last else 4
            if service == COV:
                header = bytes((0x08 | more, 5, invoke, seq, WINDOW, service))
            else:
                header = bytes((0x38 | more, invoke, seq, WINDOW, service))
            sent_at = self.send(header + chunk)
            if seq == 0:
                first_send = sent_at
            self.counts[f"tx_segment_{name}"] += 1
            if seq % WINDOW == 0 or last:
                ack = self.receive()
                require(ack == bytes((0x41 if service == COV else 0x40, invoke, seq, WINDOW)),
                        f"unexpected {name} SegmentACK at {seq}: {ack.hex()}")
                self.acks[name].append(seq)
                self.counts[f"rx_segment_ack_{name}"] += 1
        if service == COV:
            require(self.receive() == bytes((0x20, invoke, COV)), "expected admitted COV SimpleACK")
            self.counts["rx_simple_ack_cov"] += 1
        return first_send

    def run_round(self, seed, number):
        remaining = {RP, RPM}
        order = []
        while remaining:
            service, invoke = self.request(remaining)
            if not order:
                # Incoming confirmed COV reuses the live outgoing invoke ID.
                cov_send = self.transfer(COV, invoke, service_data(COV, seed, number))
            order.append(NAMES[service])
            self.transfer(service, invoke, service_data(service, seed, number))
            remaining.remove(service)
        return {"round": number, "cov_send_ns": cov_send, "read_order": order}


def main():
    endpoint_port, seed, total = map(int, sys.argv[1:])
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as link:
        link.bind(("127.0.0.1", 0))
        link.connect(("127.0.0.1", endpoint_port))
        link.settimeout(30)  # bound cleanup when the endpoint exits during an exchange
        router = Router(link)
        emit({"port": link.getsockname()[1]})
        for number in range(total):
            router.counts = Counter(rx_error_reject_abort=0)
            router.acks = {}
            try:
                result = router.run_round(seed, number)
            except Exception as error:
                emit({"round": number, "error": str(error), "counts": router.counts, "acks": router.acks})
                return 1
            emit({**result, "counts": router.counts, "acks": router.acks})
    return 0


if __name__ == "__main__":
    sys.exit(main())
