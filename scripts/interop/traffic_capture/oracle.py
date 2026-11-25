"""Expected transactions for a fixed, direct, normal-mode loopback workload."""
from dataclasses import dataclass
import socket

IP = "127.0.0.1"
CLIENT, PEER = 47810, 47811
MARKER, SINK = 32001, 32002
DEVICE = 200
PROCESS = 7
TAGS = bytes.fromhex
AI = TAGS("00000001")
PROPERTY = TAGS("0c00000001 191c")
RPM_REQUEST = TAGS("0c00000001 1e 0955 091c 1f")
DESC = "Ω" * 250
UCS4 = b"\x75\xfe" + (1 + len(DESC) * 4).to_bytes(2, "big") + b"\x03" + DESC.encode("utf-32be")


def frame(apdu, expecting=False):
    npdu = bytes((1, 4 if expecting else 0)) + apdu
    return b"\x81\x0a" + (4 + len(npdu)).to_bytes(2, "big") + npdu


@dataclass(frozen=True)
class Packet:
    name: str
    src: int
    dst: int
    raw: bytes
    invoke_key: str = ""
    invoke_at: int = 0
    establishes: bool = False

    def materialize(self, invokes):
        if not self.invoke_key:
            return self.raw
        i = self.invoke_at
        return self.raw[:i] + bytes((invokes[self.invoke_key],)) + self.raw[i + 1:]

    def match(self, src, dst, payload, invokes):
        if (src, dst) != (self.src, self.dst):
            raise ValueError(f"{self.name}: expected UDP {self.src}->{self.dst}, got {src}->{dst}")
        if self.establishes and len(payload) > self.invoke_at:
            invokes[self.invoke_key] = payload[self.invoke_at]
        wanted = self.materialize(invokes)
        if payload != wanted:
            raise ValueError(f"{self.name}: expected {wanted.hex()}, received {payload.hex()}")


def scenario(owner, nonce):
    def marker(name):
        return Packet(name, MARKER, SINK, f"RUSTY-CAPTURE/{nonce}/{name}".encode())

    # Confirmed request headers express the client's accepted response APDU size.
    max_response = 5 if owner == "standalone" else 3

    def request(name, service, body):
        apdu = bytes((2, max_response, 0, service)) + body
        return Packet(name, CLIENT, PEER, frame(apdu, True), name, 8, True)

    def reply(name, key, service, body=None):
        pdu = bytes((0x20 if body is None else 0x30, 0, service)) + (body or b"")
        return Packet(name, PEER, CLIENT, frame(pdu), key, 7)

    def peer(name, apdu, expecting=False):
        return Packet(name, PEER, CLIENT, frame(apdu, expecting))

    def client(name, apdu):
        return Packet(name, CLIENT, PEER, frame(apdu))

    sub = b"\x09" + bytes([PROCESS]) + b"\x1c" + AI
    small = TAGS("73006f6b")
    cov_header = TAGS("0907 1c020000c8 2c00000001 393c 4e 091c 2e")
    body = cov_header + UCS4 + TAGS("2f4f")
    plan = [marker("start"), marker("idle_start"), marker("idle_end"),
            # Discovery asks the local network number once on first need while it is unknown.
            # This is a local BVLC broadcast to the owner's configured loopback broadcast port.
            Packet("first_discovery_network_number", CLIENT, CLIENT, TAGS("810b0007018012")),
            client("who_is", TAGS("1008 09c8 19c8")),
            peer("i_am", TAGS("1000 c4020000c8 2205c4 9101 212a")),
            client("second_who_is", TAGS("1008 09c8 19c8")),
            peer("second_i_am", TAGS("1000 c4020000c8 2205c4 9101 212a")),
            request("rp", 12, PROPERTY), reply("rp_ack", "rp", 12, PROPERTY + b"\x3e" + small + b"\x3f"),
            request("rpm", 14, RPM_REQUEST),
            reply("rpm_ack", "rpm", 14, TAGS("0c00000001 1e 2955 4e 4441ac0000 4f 291c 4e") + small + TAGS("4f1f")),
            request("subscribe", 5, sub + TAGS("2901 393c")), reply("sub_ack", "subscribe", 5),
            request("renew", 5, sub + TAGS("2901 395a")), reply("renew_ack", "renew", 5)]
    pieces = [body[i:i + 300] for i in range(0, len(body), 300)]
    assert len(pieces) == 4
    for seq, piece in enumerate(pieces):
        more = seq < 3
        # A sender waits after segment 0, after the next full window, and at the end.
        plan.append(peer(f"cov{seq}", bytes((0x0c if more else 0x08, 5, 71, seq, 2, 1)) + piece,
                         expecting=seq in (0, 2, 3)))
        if seq in (0, 2, 3):
            plan.append(client(f"segment_ack{seq}", bytes((0x41, 71, seq, 2))))
    plan.extend([client("cov_ack", TAGS("204701")), request("unsubscribe", 5, sub),
                 reply("cancel_ack", "unsubscribe", 5),
                 peer("late", TAGS("00054801") + cov_header + small + TAGS("2f4f"), True),
                 client("unknown_subscription", TAGS("504801 9105 914f")), marker("end")])
    return plan


def udp_socket(port):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind((IP, port))
    return sock
