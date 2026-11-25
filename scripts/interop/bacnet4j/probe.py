"""Drive regular SDK calls against the Java device; save the wire evidence even on failure."""

import argparse
import asyncio
from contextlib import asynccontextmanager
import hashlib
from importlib import import_module, metadata
import json
from pathlib import Path
import platform
import sys
import zipfile

import rusty_bacnet as rb
from trace import Relay, check, messages, transfer


PEER = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 9001)
DESCRIPTION = rb.PropertyIdentifier.DESCRIPTION


async def java(command, report):
    reader, writer = await asyncio.open_connection("127.0.0.1", 47811)
    try:
        writer.write((command + "\n").encode("ascii"))
        await writer.drain()
        answer = (await asyncio.wait_for(reader.readline(), 35)).decode("ascii").strip()
        report["commands"].append({"request": command, "reply": answer})
        check(answer and not answer.startswith("FAILED"), f"Java command {command}: {answer}")
        return answer
    finally:
        writer.close()
        await writer.wait_closed()


@asynccontextmanager
async def client_role(shared):
    opts = {"interface": "127.0.0.1", "port": 0, "apdu_timeout_ms": 30000}
    if shared:
        endpoint = rb.BipEndpoint(device_instance=100, max_apdu=480, apdu_retries=0, segmentation=rb.Segmentation.SEGMENTED_RECEIVE,
                                  broadcast_address="127.255.255.255", **opts)
        await endpoint.start()
        try:
            yield await endpoint.client(confirmed_cov_subscription_limit=1)
        finally:
            await endpoint.close()
    else:
        async with rb.BACnetClient(**opts) as client:
            yield client


async def reads(client, relay, address, result, shared):
    text = "D" * 6000
    for service in (12, 14):
        start = len(relay.frames)
        if service == 12:
            values = [await client.read_property(address, PEER, DESCRIPTION)]
        else:
            selectors = [(DESCRIPTION, None), (DESCRIPTION, None)]
            obj, = await client.read_property_multiple(address, [(PEER, selectors)])
            check(obj["object_id"] == PEER, "RPM object identity")
            rows = obj["results"]
            check([(row["property_id"], row["array_index"]) for row in rows] == selectors, "RPM selector occurrences")
            check(all(row["error"] is None for row in rows), "RPM remote property error")
            values = [row["value"] for row in rows]
        check(all(value.tag == "character_string" and value.value == text for value in values), "incorrect description")
        await asyncio.wait_for(relay.finish_read(start, service), 10)
        receipt, _ = transfer(relay.frames[start:], service)
        receipt["description_count"] = len(values)
        receipt["description_sha256"] = hashlib.sha256(text.encode("ascii")).hexdigest()
        check(receipt["advertised_max_apdu"] == (480 if shared else 1476), "unexpected advertised read budget")
        result["reads"].append(receipt)


async def confirmed_cov(client, relay, address, result, report):
    events = await client.cov_notifications()
    lease = await client.expect_confirmed_cov(address, 7, PEER, initiating_device_instance=9001)
    try:
        start = len(relay.frames)
        reply = await java(f"DISCOVER {relay.port}", report)
        check(reply == "IAM 100 480 2", "Java did not learn the shared endpoint's receive capability")
        discovery = messages(relay.frames[start:])
        check(any(side == "java" and p.startswith(b"\x10\x08") for side, p in discovery)
              and any(side == "native" and p.startswith(b"\x10\x00") for side, p in discovery),
              "expected directed Who-Is and I-Am on relay")
        result["discovery"] = reply
        for process, text in ((9, "U" * 6000), (7, "A" * 6000)):
            start = len(relay.frames)
            reply = await java(f"COV {process}", report)
            receipt, body = transfer(relay.frames[start:], 1, confirmed=True)
            terminal = bytes.fromhex(receipt["terminal_apdu_hex"])
            if process == 9:
                check(reply == "ERROR 5 79" and terminal == bytes((0x50, receipt["invoke"], 1, 0x91, 5, 0x91, 79)),
                      "unexpected unknown-subscription terminal")
            else:
                check(reply == "SIMPLE_ACK" and terminal == bytes((0x20, receipt["invoke"], 1)), "expected COV SimpleACK")
                event = await asyncio.wait_for(anext(events), 10)
                check((event.subscriber_process_identifier, event.delivery, event.initiating_device_identifier,
                       event.monitored_object_identifier, event.source_network, event.source_mac) ==
                      (7, "confirmed", PEER, PEER, None, b"\x7f\x00\x00\x01" + relay.port.to_bytes(2, "big")),
                      "COV source, event order or registration")
                row, = event.values
                check((row["property_id"], row["array_index"], row["value"].value) == (DESCRIPTION, None, text),
                      "incorrect admitted COV description")
                prefix = bytes.fromhex("4e091c2e")
                check(body.count(prefix) == 1 and body.endswith(bytes.fromhex("2f4f")), "unexpected COV list framing")
                captured = body[body.index(prefix) + len(prefix):-2]
                check(row["raw"] == captured, "COV raw property differs from capture")
                receipt["raw_value_sha256"] = hashlib.sha256(captured).hexdigest()
                check(events.missed == 0, "COV iterator reported loss")
            result["cov"].append({"process": process, **receipt})
    finally:
        lease.close()


async def qualify(args, report):
    version, *files = (await java("INFO", report)).split("|")
    classpath = {relative: checksum for checksum, relative in (item.split(None, 1) for item in files)}
    check(version.startswith("JDK 17."), "expected Java 17")
    check(classpath.get("target/dependency/bacnet4j-6.1.1.jar") ==
          "588dc6e986dd118fee24d436420cbbb745aa1a7d9e8583e15e2d93deeb49e97f", "BACnet4J JAR differs")
    report["java"] = {"runtime": version, "classpath_sha256": classpath}
    for shared in (False, True):
        relay = Relay()
        loop = asyncio.get_running_loop()
        transport, _ = await loop.create_datagram_endpoint(lambda: relay, local_addr=("127.0.0.1", 0))
        result = {"owner": "shared" if shared else "standalone", "reads": [], "cov": [], "frames": relay.frames}
        report["owners"].append(result)
        try:
            async with client_role(shared) as client:
                address = f"127.0.0.1:{relay.port}"
                await reads(client, relay, address, result, shared)
                if shared and not args.reads_only:
                    await confirmed_cov(client, relay, address, result, report)
        finally:
            transport.close()


def file_hash(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--peer-image", required=True)
    parser.add_argument("--client-image", required=True)
    parser.add_argument("--reads-only", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    fixture = Path(__file__).resolve().parent
    report = {"success": False, "peer_image": args.peer_image, "client_image": args.client_image,
              "sdk_source_sha_declared": args.source_sha, "sdk_version": metadata.version("ace-rusty-bacnet"),
              "python": sys.version, "platform": platform.platform(), "reads_only": args.reads_only,
              "fixture_sha256": {str(path.relative_to(fixture)): file_hash(path) for path in
                                 (fixture / name for name in ("Dockerfile", "pom.xml", "src/main/java/SegmentedPeer.java",
                                                              "probe.py", "trace.py", "run.sh"))},
              "commands": [], "owners": []}
    try:
        native = Path(import_module("rusty_bacnet.rusty_bacnet").__file__)
        report["wheel"] = args.wheel.name
        report["wheel_sha256"] = file_hash(args.wheel)
        report["native_sha256"] = file_hash(native)
        with zipfile.ZipFile(args.wheel) as archive, archive.open("rusty_bacnet/" + native.name) as stream:
            check(hashlib.file_digest(stream, "sha256").hexdigest() == report["native_sha256"], "installed binary mismatch")
        asyncio.run(asyncio.wait_for(qualify(args, report), 150))
        report["success"] = True
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    if not report["success"]:
        print(report["failure"], file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
