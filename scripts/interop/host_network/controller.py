"""Independent BACpypes3 codec controller; only the named test devices are queried."""

from contextlib import ExitStack
from importlib import metadata
from ipaddress import IPv4Interface
import json
import socket
import sys
import time

from bacpypes3.apdu import APDU, IAmRequest, ReadPropertyACK, ReadPropertyRequest, WhoIsRequest
from bacpypes3.pdu import PDU
from bacpypes3.primitivedata import CharacterString

from common import emit, failure, identity


def preflight(config):
    addresses = [IPv4Interface(item["cidr"]) for item in config["endpoints"]]
    binds = dict.fromkeys([str(item.ip) for item in addresses] +
                          [str(item.network.broadcast_address) for item in addresses] + ["255.255.255.255"])
    opened = []
    with ExitStack() as stack:
        for addr in binds:
            sock = stack.enter_context(socket.socket(socket.AF_INET, socket.SOCK_DGRAM))
            try:
                sock.bind((addr, config["port"]))
            except OSError as error:
                return {"ok": False, "bind": [addr, config["port"]], "error": failure(error), "opened": opened}
            opened.append(list(sock.getsockname()))
    return {"ok": True, "exclusive_binds": opened}


def request(sock, target, apdu, *, broadcast):
    encoded = bytes(apdu.encode().encode().pduData)
    body = bytes((1, 0 if broadcast else 4)) + encoded
    frame = bytes((0x81, 11 if broadcast else 10)) + (len(body) + 4).to_bytes(2, "big") + body
    sock.sendto(frame, target)
    return {"destination": list(target), "udp_hex": frame.hex()}


def receive(sock, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("waiting for the requested B/IP response")
    sock.settimeout(remaining)
    frame, source = sock.recvfrom(4096)
    if frame[:2] != b"\x81\x0a" or int.from_bytes(frame[2:4], "big") != len(frame) or frame[4:6] != b"\x01\x00":
        raise ValueError("unexpected BVLC/NPDU response")
    return APDU.decode(PDU(frame[6:])), {"source": list(source), "udp_hex": frame.hex()}


def discover(sock, config):
    by_network = {}
    for endpoint in config["endpoints"]:
        interface = IPv4Interface(endpoint["cidr"])
        by_network.setdefault(str(interface.network.broadcast_address), {})[endpoint["instance"]] = str(interface.ip)
    queries = []
    for broadcast, targets in by_network.items():
        ids = sorted(targets)
        groups = [ids] if ids[-1] - ids[0] == len(ids) - 1 else [[instance] for instance in ids]
        for group in groups:
            query = WhoIsRequest(deviceInstanceRangeLowLimit=group[0], deviceInstanceRangeHighLimit=group[-1])
            sent = request(sock, (broadcast, config["port"]), query, broadcast=True)
            responses = []
            pending = set(group)
            deadline = time.monotonic() + 30
            while pending:
                apdu, record = receive(sock, deadline)
                iam = IAmRequest.decode(apdu)
                instance = int(iam.iAmDeviceIdentifier[1])
                if instance not in pending or record["source"] != [targets[instance], config["port"]]:
                    raise ValueError(f"unexpected I-Am instance {instance} from {record['source']}")
                pending.remove(instance)
                responses.append({**record, "instance": instance})
            queries.append({"instances": group, "sent": sent, "responses": responses})
    return {"ok": True, "queries": queries}


def read_name(sock, config, index, invoke):
    endpoint = config["endpoints"][index]
    host = str(IPv4Interface(endpoint["cidr"]).ip)
    oid = ("device", endpoint["instance"])
    read = ReadPropertyRequest(objectIdentifier=oid, propertyIdentifier="object-name")
    read.apduInvokeID, read.apduMaxResp, read.apduMaxSegs = invoke, 3, 0
    read.apduSeg = read.apduMor = read.apduSA = False
    sent = request(sock, (host, config["port"]), read, broadcast=False)
    apdu, received = receive(sock, time.monotonic() + 30)
    answer = ReadPropertyACK.decode(apdu)
    if (received["source"] != [host, config["port"]] or answer.apduInvokeID != invoke
            or answer.objectIdentifier != read.objectIdentifier or answer.propertyIdentifier != read.propertyIdentifier
            or answer.propertyArrayIndex is not None):
        raise ValueError("ReadProperty source, invoke or selector mismatch")
    return {"ok": True, "name": str(answer.propertyValue.cast_out(CharacterString)), "instance": endpoint["instance"],
            "sent": sent, "received": received}


def main():
    config = json.loads(sys.argv[1])
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sock.bind((str(IPv4Interface(config["controller"]).ip), 0))
        emit({"identity": identity(), "source": list(sock.getsockname()), "bacpypes3": metadata.version("bacpypes3")})
        for line in sys.stdin:
            command = json.loads(line)
            try:
                if command["op"] == "close":
                    emit({"closed": True})
                    return
                if command["op"] == "preflight":
                    result = preflight(config)
                elif command["op"] == "discover":
                    result = discover(sock, config)
                elif command["op"] == "name":
                    result = read_name(sock, config, command["index"], command["invoke"])
                else:
                    raise ValueError("unknown controller command")
                emit(result)
            except Exception as error:
                emit({"ok": False, "error": failure(error)})


if __name__ == "__main__":
    main()
