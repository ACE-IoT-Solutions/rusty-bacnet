"""Run inside the SUT or a single-link peer. IPC uses Podman stdio only."""
import asyncio
from contextlib import AsyncExitStack
import fcntl
import hashlib
import importlib.metadata
import ipaddress
import json
import logging
import os
from pathlib import Path
import socket
import struct
import sys
import time
import traceback
import zipfile

PORT = 47808
PEER_PORT = 47900
PHASES = ("limited", "directed", "unicast")


def check(ok, why):
    if not ok:
        raise ValueError(why)


def emit(obj):
    print(json.dumps(obj), flush=True)


def identity():
    uid_map = Path("/proc/self/uid_map").read_text()
    return dict(uid=os.getuid(), uid_map=uid_map, namespace=os.readlink("/proc/self/ns/net"),
                user_namespace=os.readlink("/proc/self/ns/user"), python=sys.version)


def interfaces():
    result = {}
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        for index, name in socket.if_nameindex():
            if name == "lo":
                continue
            encoded = struct.pack("256s", name.encode())

            def address(request):
                return socket.inet_ntoa(fcntl.ioctl(sock, request, encoded)[20:24])

            result[name] = dict(index=index, ip=address(0x8915), mask=address(0x891b),
                                broadcast=address(0x8919))
    return result


def resolved(config, key):
    actual = interfaces()
    result = []
    for link in config:
        matches = [(name, nic) for name, nic in actual.items() if nic["ip"] == link[key]]
        (name, nic), = matches
        subnet = ipaddress.IPv4Network(link["subnet"])
        check(nic["mask"] == str(subnet.netmask) and nic["broadcast"] == str(subnet.broadcast_address)
              and nic["index"] > 0, f"interface mismatch: {nic}")
        result.append(dict(name=name, **nic))
    check(len(result) == len(actual), f"unexpected additional interface: {actual}")
    return result


def bvll(function, body):
    return struct.pack(">BBH", 0x81, function, 4 + len(body)) + body


def device(instance):
    return struct.pack(">I", (8 << 22) | instance)


def peer(config, index, phase):
    link = config[index]
    nic, = resolved([link], "peer")
    step = PHASES.index(phase)
    target = ("255.255.255.255", nic["broadcast"], link["sut"])[step]
    original = 0x0a if phase == "unicast" else 0x0b
    instance = 12000 + 100 * index + step
    value = 10 * (index + 1) + step
    packets = []
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sock.bind((nic["ip"], PEER_PORT + step))
        sock.settimeout(20)

        def send(destination, data):
            packets.append(dict(source=list(sock.getsockname()), destination=[destination, PORT], hex=data.hex()))
            # Linux IP_PKTINFO selects the actual link for limited broadcast on an internal bridge.
            info = struct.pack("=I4s4s", nic["index"], socket.inet_aton(nic["ip"]), bytes(4))
            sock.sendmsg([data], [(socket.IPPROTO_IP, 8, info)], 0, (destination, PORT))

        send(target, bvll(original, bytes.fromhex("0100 1000 c4") + device(instance)
                          + bytes.fromhex("2205c4 9103 212a")))
        send(target, bvll(original, bytes.fromhex("0100 1002 0907 1c") + device(instance)
                          + bytes.fromhex("2c00000001 393c 4e09552e21") + bytes([value]) + bytes.fromhex("2f4f")))
        if phase == "unicast":
            invoke = 45 + index
            ref = b"\x0c" + device(link["instance"]) + bytes.fromhex("194d")
            send(target, bvll(10, bytes.fromhex("0104 0003") + bytes([invoke, 12]) + ref))
            reply, sender = sock.recvfrom(2048)
            packets.append(dict(source=list(sender), destination=list(sock.getsockname()), hex=reply.hex()))
            name = link["name"].encode()
            ack = (bytes.fromhex("0100 30") + bytes([invoke, 12]) + ref
                   + bytes([0x3e, 0x75, len(name) + 1, 0]) + name + b"\x3f")
            check(sender == (link["sut"], PORT) and reply == bvll(10, ack),
                  f"unexpected RP Object_Name reply from {sender}: {reply.hex()}")
    return dict(identity=identity(), interface=nic, phase=phase, packets=packets)


class WrongInterface(logging.Handler):
    def __init__(self):
        super().__init__(logging.WARNING)
        self.records = []

    def emit(self, record):
        if "Dropping a broadcast that arrived on another interface" in record.getMessage():
            self.records.append(dict(time=time.monotonic(), target=record.bacnet_target,
                                     level=record.levelname, fields=record.bacnet_fields))


async def sut(config, wheel):
    import rusty_bacnet as rb
    from importlib import import_module

    nics = resolved(config, "sut")
    check(nics[0]["index"] != nics[1]["index"], "SUT interfaces have the same index")
    native = Path(import_module("rusty_bacnet.rusty_bacnet").__file__)
    digest = hashlib.sha256(native.read_bytes()).hexdigest()
    with zipfile.ZipFile(wheel) as archive:
        check(hashlib.sha256(archive.read("rusty_bacnet/" + native.name)).hexdigest() == digest,
              "installed native extension differs from wheel")
    logger = logging.getLogger("rusty_bacnet")
    logger.setLevel(logging.WARNING)
    warnings = WrongInterface()
    logger.addHandler(warnings)
    rb.refresh_logging()
    endpoints, clients, streams = [], [], []
    events = [[], []]
    tables = [[], []]
    health = []

    async def admissions():
        for i, client in enumerate(clients):
            rows = await client.discovered_devices()
            tables[i] = sorted([row.object_identifier.instance, bytes(row.mac_address).hex()] for row in rows)

    def observe(i, event):
        row, = event.values
        check(event.delivery == "unconfirmed" and event.subscriber_process_identifier == 7
              and event.monitored_object_identifier == rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
              and row["property_id"] == rb.PropertyIdentifier.PRESENT_VALUE, f"unexpected event: {event!r}")
        events[i].append(dict(instance=event.initiating_device_identifier.instance, source=event.source_mac.hex(),
                              value=row["value"].value, raw=row["raw"].hex()))

    async with AsyncExitStack() as stack:
        for link, nic in zip(config, nics):
            ep = rb.BipEndpoint(link["instance"], device_name=link["name"], interface=nic["name"], port=PORT,
                                broadcast_address=nic["broadcast"], share_port_by_address=True)
            await stack.enter_async_context(ep)
            endpoints.append(ep)
            check(await ep.local_address() == f"{nic['ip']}:{PORT}", "wrong named-interface bind")
            client = await ep.client()
            clients.append(client)
            streams.append(await client.cov_notifications())
            health.append((await ep.status())["transport_health"])
        emit(dict(ready=True, identity=identity(), interfaces=nics, health=health, native_sha256=digest,
                  sdk=importlib.metadata.version("ace-rusty-bacnet"), warnings=list(warnings.records)))
        for step, phase in enumerate(PHASES):
            command = await asyncio.to_thread(sys.stdin.readline)
            check(command.strip() == phase, f"expected checkpoint {phase}")
            try:
                async with asyncio.timeout(20):
                    for i, stream in enumerate(streams):
                        observe(i, await anext(stream))
                    # A routed table update and Python log delivery have independent workers.
                    while True:
                        await admissions()
                        if (all(len(table) >= step + 1 for table in tables)
                                and (phase != "limited" or len(warnings.records) >= 2)):
                            break
                        await asyncio.sleep(0.025)
            except TimeoutError:
                emit(dict(error=f"admission checkpoint timed out: {phase}", tables=tables, events=events,
                          warnings=warnings.records))
                raise
            emit(dict(phase=phase, tables=tables, events=events, warnings=list(warnings.records)))
        check((await asyncio.to_thread(sys.stdin.readline)).strip() == "close", "expected close")
        await admissions()
        final_health = [(await ep.status())["transport_health"] for ep in endpoints]
    for i, stream in enumerate(streams):
        async for event in stream:
            observe(i, event)
    emit(dict(closed=True, tables=tables, events=events, missed=[stream.missed for stream in streams],
              warnings=warnings.records, health=final_health))


def main():
    role, config = sys.argv[1], json.loads(sys.argv[2])
    if role == "peer":
        emit(peer(config, int(sys.argv[3]), sys.argv[4]))
    else:
        asyncio.run(sut(config, Path(sys.argv[3])))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        emit(dict(error=traceback.format_exc()))
        sys.exit(1)
