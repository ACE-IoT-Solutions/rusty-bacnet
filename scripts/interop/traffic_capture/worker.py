"""Exercise one SDK owner in an isolated namespace and retain the kernel capture."""
import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import sys
import traceback
import zipfile

from oracle import CLIENT, DESC, DEVICE, IP, MARKER, PEER, PROCESS, SINK, UCS4, scenario, udp_socket
from pcap import audit, records, udp

OUT = Path("/tmp/traffic-output")
PCAP = OUT / "loopback.pcap"


def check(ok, why):
    if not ok:
        raise ValueError(why)


def observation(report, name, value):
    report.setdefault("observations", {})[name] = value


async def visible(step):
    # -U flushes complete packets. A file can end mid-record while tcpdump is writing.
    async with asyncio.timeout(10):
        while True:
            for record in records(PCAP.read_bytes(), partial=True):
                if record.data.endswith(step.raw) and udp(record.data) == (MARKER, SINK, step.raw):
                    return
            await asyncio.sleep(0.01)


async def exchange(args, report, markers, actions):
    import rusty_bacnet as rb

    addr = f"{IP}:{PEER}"
    options = dict(interface=IP, port=CLIENT, broadcast_address=IP, apdu_timeout_ms=30_000)
    ai = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
    prop = rb.PropertyIdentifier
    shared = args.owner == "shared"
    owner = (rb.BipEndpoint(**options, device_instance=100, max_apdu=480, apdu_retries=0,
                            segmentation=rb.Segmentation.SEGMENTED_RECEIVE)
             if shared else rb.BACnetClient(**options, confirmed_cov_subscription_limit=1))
    events = lease = None
    invokes = {}
    loop = asyncio.get_running_loop()
    with udp_socket(PEER) as peer:
        peer.setblocking(False)

        async def receive(name):
            step = actions[name]
            raw, source = await loop.sock_recvfrom(peer, 65535)
            check(source[0] == IP, f"peer received from {source}")
            step.match(source[1], PEER, raw, invokes)

        def respond(name):
            peer.sendto(actions[name].materialize(invokes), (IP, CLIENT))

        try:
            await owner.__aenter__()
            client = await owner.client(confirmed_cov_subscription_limit=1) if shared else owner
            events = await client.cov_notifications()
            arrivals = await client.i_am_arrivals()
            markers("idle_start")
            if shared:
                status = await owner.status()
                check(status["active_leases"] == 0 and status["transport_health"]["ready"], "shared status")
                observation(report, "transport_health", status["transport_health"])
            check(not await client.discovered_devices(), "expected empty initial device table")
            expectation = await client.expect_confirmed_cov(addr, 8, ai)
            expectation.close()
            if args.inject:
                with udp_socket(32003) as stray, udp_socket(32004) as bound_sink:
                    stray.sendto(b"deliberately unexpected UDP", (IP, 32004))
                    bound_sink.recv(128)
            await asyncio.sleep(args.idle)
            markers("idle_end")

            for query, response in (("who_is", "i_am"), ("second_who_is", "second_i_am")):
                await client.who_is(low=DEVICE, high=DEVICE, scope=addr)
                await receive(query)
                respond(response)
                arrived = await anext(arrivals)
                check(arrived.object_identifier == rb.ObjectIdentifier(rb.ObjectType.DEVICE, DEVICE)
                      and arrived.link_sender == addr, f"{response} identity or link sender")
            value = client.read_property(addr, ai, prop.DESCRIPTION)
            await receive("rp")
            respond("rp_ack")
            value = await value
            check(value.value == "ok" and value.read_context.raw == b"\x73\0ok", "read property")
            rows = client.read_property_multiple(addr, [(ai, [(prop.PRESENT_VALUE, None), (prop.DESCRIPTION, None)])])
            await receive("rpm")
            respond("rpm_ack")
            (obj,) = await rows
            check(obj["object_id"] == ai and [(r["property_id"], r["value"].value) for r in obj["results"]]
                  == [(prop.PRESENT_VALUE, 21.5), (prop.DESCRIPTION, "ok")], "read property multiple")
            lease = await client.expect_confirmed_cov(addr, PROCESS, ai, initiating_device_instance=DEVICE)
            for name, response, life in [("subscribe", "sub_ack", 60), ("renew", "renew_ack", 90)]:
                sent = client.subscribe_cov(addr, PROCESS, ai, True, life)
                await receive(name)
                respond(response)
                await sent
            for seq in range(4):
                respond(f"cov{seq}")
                if seq in (0, 2, 3):
                    await receive(f"segment_ack{seq}")
            await receive("cov_ack")
            event = await anext(events)
            (row,) = event.values
            check(event.subscriber_process_identifier == PROCESS and event.monitored_object_identifier == ai
                  and row["property_id"] == prop.DESCRIPTION and row["value"].value == DESC
                  and row["raw"] == UCS4, "admitted COV identity, value or original octets")
            observation(report, "cov_value", {"text_characters": len(DESC), "raw_bytes": len(UCS4),
                                              "raw_sha256": hashlib.sha256(row["raw"]).hexdigest()})
            sent = client.unsubscribe_cov(addr, PROCESS, ai)
            await receive("unsubscribe")
            respond("cancel_ack")
            await sent
            lease.close()
            respond("late")
            await receive("unknown_subscription")
            check(events.missed == 0, "COV observer missed an event")
            if shared:
                status = await owner.status()
                check(status["transport_health"]["ready"] and status["active_leases"] == 0, "ending health")
        finally:
            if lease is not None:
                lease.close()
            try:
                await asyncio.wait_for(owner.close() if shared else owner.stop(), 20)
            except Exception as error:
                report["cleanup_errors"].append(f"SDK shutdown: {error!r}")
        # Closing the only owner ends the iterator after all published events have drained.
        try:
            await anext(events)
        except StopAsyncIteration:
            observation(report, "accepted_events", 1)
        else:
            raise ValueError("closed-lease notification was published")


def installed_wheel(path):
    dist = importlib.metadata.distribution("ace-rusty-bacnet")
    with zipfile.ZipFile(path) as archive:
        (extension,) = [n for n in archive.namelist() if n.endswith(".so")]
        digest = hashlib.sha256(archive.read(extension)).hexdigest()
    installed = dist.locate_file(extension)
    check(hashlib.sha256(installed.read_bytes()).hexdigest() == digest, "installed native differs from wheel")
    return dict(version=dist.version, native=str(installed), native_sha256=digest, python=sys.version,
                python_executable=sys.executable)


async def run(args, report):
    report["sdk"] = installed_wheel(args.wheel)
    report["interfaces"] = socket.if_nameindex()
    check([name for _, name in report["interfaces"]] == ["lo"], "capture requires an isolated lo-only namespace")
    report["network_namespace"] = os.readlink("/proc/self/ns/net")
    report["uid_map"] = Path("/proc/self/uid_map").read_text()
    report["uid"] = os.getuid()
    report["effective_capabilities"] = next(line.split()[1] for line in Path("/proc/self/status").read_text().splitlines()
                                            if line.startswith("CapEff:"))
    check(int(report["effective_capabilities"], 16) == 1 << 13, "expected only CAP_NET_RAW")
    report["packages"] = subprocess.check_output(["dpkg-query", "-W", "tcpdump", "libpcap0.8t64"], text=True).splitlines()
    report["tcpdump_version"] = subprocess.check_output(["tcpdump", "--version"], text=True).strip()
    nonce = secrets.token_hex(16)
    report.update(nonce=nonce, owner=args.owner, requested_idle_seconds=args.idle, inject=args.inject)
    actions = {p.name: p for p in scenario(args.owner, nonce)}
    command = ["tcpdump", "-i", "lo", "-p", "--immediate-mode", "-U", "-s", "0", "-w", str(PCAP), "-Z", "root"]
    report["tcpdump_command"] = command
    with udp_socket(MARKER) as sender, udp_socket(SINK) as sink:
        sink.settimeout(10)
        def marker(name):
            sender.sendto(actions[name].raw, (IP, SINK))
            check(sink.recv(512) == actions[name].raw, "boundary marker was not delivered")

        capture = await asyncio.create_subprocess_exec(*command, stderr=asyncio.subprocess.PIPE,
                                                       env={**os.environ, "LC_ALL": "C"})
        banner = b""
        try:
            async with asyncio.timeout(10):
                while b"listening on lo," not in banner:
                    line = await capture.stderr.readline()
                    check(line, "tcpdump exited before opening lo")
                    banner += line
            marker("start")
            await visible(actions["start"])
            try:
                async with asyncio.timeout(90):
                    await exchange(args, report, marker, actions)
            except Exception:
                report["error"] = traceback.format_exc()
            marker("end")
            await visible(actions["end"])
        except Exception:
            report["capture_error"] = traceback.format_exc()
        finally:
            if capture.returncode is None:
                capture.send_signal(signal.SIGINT)
            try:
                _, rest = await asyncio.wait_for(capture.communicate(), 10)
            except TimeoutError:
                capture.kill()
                _, rest = await capture.communicate()
                report["cleanup_errors"].append("tcpdump needed SIGKILL")
            report["tcpdump_stderr"] = (banner + rest).decode(errors="replace")
            report["tcpdump_exit"] = capture.returncode
    drops = re.findall(r"^(\d+) packets? dropped by kernel$", report["tcpdump_stderr"], re.M)
    check(capture.returncode == 0 and drops == ["0"], "missing clean capture exit/zero kernel-drop statistics")
    report["kernel_drops"] = 0
    raw = PCAP.read_bytes()
    report["pcap_sha256"] = hashlib.sha256(raw).hexdigest()
    count = len(records(raw))
    counts = re.findall(r"^(\d+) packets? captured$", report["tcpdump_stderr"], re.M)
    check(counts == [str(count)], "tcpdump captured count differs from complete records on disk")
    report["captured_records"] = count
    report["audit"] = audit(raw, args.owner, nonce, args.idle)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("owner", choices=["standalone", "shared"])
    parser.add_argument("wheel", type=Path)
    parser.add_argument("idle", type=float)
    parser.add_argument("--inject", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(exist_ok=True)
    report = {"cleanup_errors": []}
    try:
        asyncio.run(run(args, report))
    except Exception:
        key = "audit_error" if "pcap_sha256" in report else "capture_error"
        report[key] = traceback.format_exc()
    report["success"] = "audit" in report and not any(report.get(k) for k in
                                                       ("error", "audit_error", "capture_error", "cleanup_errors"))
    (OUT / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
