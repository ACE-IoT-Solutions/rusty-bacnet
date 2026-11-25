"""Run inside the isolated namespace created by run.py."""
import asyncio
from contextlib import ExitStack
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import socket
import struct
import sys
import time
import traceback
import zipfile

OWNER = ("127.0.0.1", 47809)
BBMD = ("127.0.0.2", 47808)
ROUTER = ("127.0.0.3", 47808)
ROGUE = ("127.0.0.4", 47808)
FAR = 2589
NETWORK = 5
STATION = b"\x0a"
STATION_FIELD = struct.pack(">HB", NETWORK, len(STATION)) + STATION
DEVICE_OID = struct.pack(">I", (8 << 22) | FAR)
PV_REFERENCE = bytes.fromhex("0c00000001 1955")
OUT = Path("/tmp/receipt.json")


def check(ok, message):
    if not ok:
        raise ValueError(message)


def address(pair):
    return "%s:%d" % pair


def mac(pair):
    return socket.inet_aton(pair[0]) + struct.pack(">H", pair[1])


def bvll(function, body):
    return struct.pack(">BBH", 0x81, function, 4 + len(body)) + body


def routed(apdu, *, forwarded=False, reply_expected=False):
    npdu = bytes([1, 0x0c if reply_expected else 0x08]) + STATION_FIELD + apdu
    return bvll(4, mac(ROUTER) + npdu) if forwarded else bvll(10, npdu)


def to_station(apdu, *, reply_expected=False, local_source=False):
    control = 0x24 if reply_expected else 0x20
    source = b""
    if local_source:
        control |= 0x08
        source = struct.pack(">HB", 7, 6) + mac(OWNER)
    return bvll(10, bytes([1, control]) + STATION_FIELD + source + b"\xff" + apdu)


def notification(value, *, confirmed=False):
    # Same monitored object and process throughout. Distinct values fence the observer.
    header = bytes.fromhex("0005 41 01") if confirmed else bytes.fromhex("1002")
    body = (bytes.fromhex("0907 1c") + DEVICE_OID + bytes.fromhex("2c00000001 393c 4e09552e44")
            + struct.pack(">f", value) + bytes.fromhex("2f4f"))
    return routed(header + body, forwarded=not confirmed, reply_expected=confirmed)


class Peers:
    def __init__(self, report, stack):
        self.report = report
        self.started = time.monotonic()
        self.phase = "initial"
        self.ttl = 4
        self.disposition = "accept"
        self.unknown = report["profile"] == "unknown"
        self.question_due = None
        self.questions = asyncio.Queue()
        self.recovery_reply_sent = False
        self.silent_registrations = 0
        self.router_queue = asyncio.Queue()
        self.sockets = {}
        loop = asyncio.get_running_loop()
        for pair in (BBMD, ROUTER, ROGUE):
            sock = stack.enter_context(socket.socket(socket.AF_INET, socket.SOCK_DGRAM))
            sock.bind(pair)
            sock.setblocking(False)
            self.sockets[pair] = sock
            loop.add_reader(sock.fileno(), self.receive, pair)
            stack.callback(loop.remove_reader, sock.fileno())

    def log(self, source, destination, data):
        self.report["packets"].append(dict(seconds=time.monotonic() - self.started, phase=self.phase,
                                           source=address(source), destination=address(destination),
                                           hex=data.hex()))

    def send(self, source, data):
        self.log(source, OWNER, data)
        self.sockets[source].sendto(data, OWNER)

    def expect_question(self, cause):
        if self.unknown:
            check(self.question_due is None, f"unanswered wire expectation: {self.question_due}")
            self.question_due = cause

    async def saw_question(self, cause):
        if self.unknown:
            observed = await asyncio.wait_for(self.questions.get(), 15)
            check(observed == cause, f"number question after {observed}, expected {cause}")

    def require_routed(self, frame, apdu, *, reply_expected=False):
        if frame == to_station(apdu, reply_expected=reply_expected):
            return
        check(not self.unknown and frame == to_station(apdu, reply_expected=reply_expected, local_source=True),
              f"wrong packet at R in {self.phase}: {frame.hex()}")

    def receive(self, peer):
        try:
            data, sender = self.sockets[peer].recvfrom(65535)
            self.log(sender, peer, data)
            check(sender == OWNER, f"unexpected sender {sender} at {peer}")
            if peer == ROUTER:
                self.router_queue.put_nowait(data)
                return
            check(peer == BBMD, "rogue forwarder received a reply")
            if data == bytes.fromhex("81090007018012"):
                check(self.question_due is not None, f"unsolicited number question in {self.phase}")
                self.report["network_number_questions"].append(self.question_due)
                self.questions.put_nowait(self.question_due)
                self.question_due = None
                return
            check(self.ttl is not None and data == bvll(5, struct.pack(">H", self.ttl)),
                  f"unexpected packet at B during {self.phase}: {data.hex()}")
            if self.disposition == "silent":
                self.silent_registrations += 1
            else:
                refused = self.disposition == "refuse_once"
                if self.phase == "recovery" and not self.recovery_reply_sent:
                    self.expect_question("accepted_after_lapse")
                    self.recovery_reply_sent = True
                self.send(BBMD, bvll(0, struct.pack(">H", 0x30 if refused else 0)))
                if refused:
                    self.disposition = "silent"
        except Exception as error:
            self.report["peer_errors"].append(str(error))

    async def registration(self, endpoint, label, condition):
        async with asyncio.timeout(20):
            while True:
                status = await endpoint.foreign_registration()
                health = (await endpoint.status())["transport_health"]
                check(health["ready"], f"{label}: receive health lost")
                if condition(status):
                    check(status["last_bbmd"] == address(BBMD), f"{label}: unexpected BBMD")
                    self.report["states"][label] = dict(seconds=time.monotonic() - self.started,
                                                        registration=status, transport_health=health)
                    return status
                check(not self.report["peer_errors"], str(self.report["peer_errors"]))
                await asyncio.sleep(0.025)


async def read(client, target, peers, rb, value):
    ai = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
    requested = client.read_property(target, ai, rb.PropertyIdentifier.PRESENT_VALUE)
    frame = await peers.router_queue.get()
    # The invoke precedes the service and exact reference in either routed NPDU layout.
    invoke = frame[-len(PV_REFERENCE) - 2]
    peers.require_routed(frame, bytes([2, 3, invoke, 12]) + PV_REFERENCE, reply_expected=True)
    raw_value = bytes((0x21, value))
    peers.send(ROUTER, routed(bytes([0x30, invoke, 12]) + PV_REFERENCE + b"\x3e" + raw_value + b"\x3f"))
    got = await requested
    check(got.value == value and got.read_context.raw == raw_value, "RP response value or octets")
    peers.report["reads"].append(dict(phase=peers.phase, invoke=invoke, value=value))


async def routed_notifications(client, peers, rb, ai):
    arrivals = await client.i_am_arrivals()
    events = await client.cov_notifications()
    i_am = bytes.fromhex("1000 c4") + DEVICE_OID + bytes.fromhex("2205c4 9103 22022b")
    peers.expect_question("first_routed_i_am")
    peers.send(BBMD, routed(i_am, forwarded=True))
    record = await anext(arrivals)
    check(record.forwarded and record.object_identifier == rb.ObjectIdentifier(rb.ObjectType.DEVICE, FAR)
          and record.link_sender == record.forwarding_bbmd == address(BBMD)
          and record.link_source == record.original_source == record.next_hop_router == address(ROUTER)
          and (record.source_network, record.source_address) == (NETWORK, STATION), f"I-Am {record!r}")
    target = rb.RoutedTarget(record.source_network, record.source_address, router=record.next_hop_router)
    peers.report["i_am"] = dict(sender=record.link_sender, source=record.link_source,
                                 router=record.next_hop_router, station=record.source_address.hex(),
                                 network=record.source_network, device=record.object_identifier.instance)
    await peers.saw_question("first_routed_i_am")
    await read(client, target, peers, rb, 11)

    async def heard(value, delivery):
        event = await anext(events)
        row, = event.values
        check(event.delivery == delivery and event.source_mac == mac(ROUTER)
              and (event.source_network, event.source_address) == (NETWORK, STATION)
              and event.monitored_object_identifier == ai and event.subscriber_process_identifier == 7
              and event.initiating_device_identifier == record.object_identifier
              and row["property_id"] == rb.PropertyIdentifier.PRESENT_VALUE
              and row["value"].value == value and row["raw"] == b"\x44" + struct.pack(">f", value),
              f"unexpected COV event {event!r} {row!r}")
        peers.report["events"].append(dict(value=value, delivery=delivery, source=event.source_mac.hex(),
                                           network=event.source_network, station=event.source_address.hex()))

    peers.phase = "forwarded_trust"
    duplicate = notification(17)
    peers.send(ROGUE, duplicate)
    peers.send(BBMD, notification(18))
    await heard(18, "unconfirmed")
    peers.send(BBMD, duplicate)
    await heard(17, "unconfirmed")
    peers.phase = "strict_routed_cov"
    lease = await client.expect_confirmed_cov(target, 7, ai, initiating_device_instance=FAR)
    try:
        peers.send(ROUTER, notification(19, confirmed=True))
        ack = await peers.router_queue.get()
        peers.require_routed(ack, bytes.fromhex("204101"))
        await heard(19, "confirmed")
    finally:
        lease.close()
    check(events.missed == 0, "COV observer loss")
    return target, events


async def exercise(report):
    import rusty_bacnet as rb

    def endpoint(ttl):
        number = dict(network_number=7, registered_network_port=1) if report["profile"] == "configured7" else {}
        return rb.BipEndpoint(device_instance=100, interface=OWNER[0], port=OWNER[1],
                              broadcast_address="127.255.255.255", **number,
                              max_apdu=480, apdu_retries=0,
                              foreign_device=rb.ForeignDevice(address(BBMD), ttl=ttl, renewal=1))

    async def close(owner):
        unwinding = sys.exception() is not None
        try:
            await asyncio.wait_for(owner.close(), 15)
        except Exception as error:
            report["cleanup_errors"].append(f"endpoint close: {error!r}")
            if not unwinding:
                raise

    def accepted(status):
        return status["registered"] and status["last_outcome"] == "accepted"

    with ExitStack() as stack:
        peers = Peers(report, stack)
        owner = endpoint(4)
        try:
            await owner.start()
            check(await owner.local_address() == address(OWNER), "original owner bound different address")
            first = await peers.registration(owner, "initial", accepted)
            await peers.registration(owner, "renewed", lambda s: accepted(s) and s["accepted"] > first["accepted"])
            client = await owner.client(confirmed_cov_subscription_limit=1)
            peers.phase = "forwarded_i_am"
            target, events = await routed_notifications(client, peers, rb,
                                                       rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1))
            peers.phase = "refused_then_silent"
            peers.disposition = "refuse_once"
            refused = await peers.registration(owner, "refused", lambda s: s["last_outcome"] == "refused")
            check(refused["last_result"] == 0x30 and refused["registered"], "previous lease after refusal")
            await peers.registration(owner, "expired", lambda s: not s["registered"] and s["lapses"] > 0)
            check(peers.silent_registrations > 0, "no renewal was withheld")
            peers.phase = "lapsed_read"
            await read(client, target, peers, rb, 22)
            check(not (await owner.foreign_registration())["registered"], "RP did not finish during lapse")
            peers.phase = "recovery"
            peers.disposition = "accept"
            await peers.registration(owner, "recovered", lambda s: accepted(s) and s["accepted"] > refused["accepted"])
            await peers.saw_question("accepted_after_lapse")
        finally:
            await close(owner)
        peers.phase = "original_closed"
        peers.ttl = None
        try:
            extra = await anext(events)
        except StopAsyncIteration:
            pass
        else:
            raise ValueError(f"extra admitted event after drain: {extra!r}")

        peers.phase = "same_bind_replacement"
        peers.ttl = 8
        replacement = endpoint(8)
        try:
            await replacement.start()
            check(await replacement.local_address() == address(OWNER), "replacement owns different address")
            await peers.registration(replacement, "replacement_initial", accepted)
            await read(await replacement.client(), target, peers, rb, 33)
            await peers.registration(replacement, "replacement_renewed", lambda s: accepted(s) and s["accepted"] >= 3)
        finally:
            await close(replacement)
        peers.phase = "after_close"
        peers.ttl = None
        before = time.monotonic()
        await asyncio.sleep(2.25)
        report["post_close_seconds"] = time.monotonic() - before
        check(peers.router_queue.empty(), "extra datagrams at R")
        check(not report["peer_errors"], str(report["peer_errors"]))
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as released:
            released.bind(OWNER)
            report["released_address"] = address(released.getsockname())


def main():
    wheel = Path(sys.argv[1])
    report = dict(profile=sys.argv[2], network_number_questions=[], packets=[], states={}, reads=[],
                  events=[], peer_errors=[], cleanup_errors=[])
    try:
        report["mounted_inputs"] = json.loads(wheel.with_name("inputs.json").read_text())
        report["namespace"] = os.readlink("/proc/self/ns/net")
        report["uid"] = os.getuid()
        report["uid_map"] = Path("/proc/self/uid_map").read_text()
        report["interfaces"] = [name for _, name in socket.if_nameindex()]
        check(report["interfaces"] == ["lo"], "expected only loopback")
        report["python"] = sys.version
        dist = importlib.metadata.distribution("ace-rusty-bacnet")
        with zipfile.ZipFile(wheel) as archive:
            name, = [name for name in archive.namelist() if name.endswith(".so")]
            expected = hashlib.sha256(archive.read(name)).hexdigest()
        native = dist.locate_file(name)
        digest = hashlib.sha256(native.read_bytes()).hexdigest()
        check(digest == expected, "installed extension differs from wheel")
        report["sdk"] = dict(version=dist.version, native=str(native), native_sha256=digest)

        async def scenario():
            async with asyncio.timeout(90):
                await exercise(report)

        asyncio.run(scenario())
    except Exception:
        report["error"] = traceback.format_exc()
    report["success"] = not any(report.get(k) for k in ("error", "cleanup_errors", "peer_errors"))
    OUT.write_text(json.dumps(report, indent=2) + "\n")
    return int(not report["success"])


if __name__ == "__main__":
    sys.exit(main())
