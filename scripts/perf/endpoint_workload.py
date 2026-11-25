#!/usr/bin/env python3
"""Measure one local shared-endpoint sample, with the UDP router in its own process."""

import argparse
import asyncio
from collections import Counter
import hashlib
from datetime import datetime, timezone
from importlib import import_module, metadata
import json
import math
import os
from pathlib import Path
import platform
import re
import resource
import socket
import sys
import sysconfig
import time
import zipfile

import endpoint_workload_peer as wire


def digest(stream):
    result = hashlib.sha256()
    for block in iter(lambda: stream.read(131072), b""):
        result.update(block)
    return result.hexdigest()


def file_hash(path):
    with Path(path).open("rb") as stream:
        return digest(stream)


def provenance(wheel):
    native = Path(import_module("rusty_bacnet.rusty_bacnet").__file__).resolve()
    member = "rusty_bacnet/" + native.name
    with zipfile.ZipFile(wheel) as archive:
        wire.require(archive.namelist().count(member) == 1, f"no unique native extension {member} in wheel")
        with archive.open(member) as stream:
            expected = digest(stream)
    installed = file_hash(native)
    return {"wheel": str(wheel.resolve()), "wheel_sha256": file_hash(wheel),
            "native_extension": str(native), "native_sha256": installed,
            "wheel_member": member, "wheel_member_sha256": expected, "sdk_version": metadata.version("ace-rusty-bacnet"),
            "harness_sha256": {Path(path).name: file_hash(path) for path in (__file__, wire.__file__)}}


def environment():
    return {"os": platform.platform(), "machine": platform.machine(), "processor": platform.processor(),
            "cpu_count": os.cpu_count(), "python": sys.version, "interpreter": sys.executable,
            "abi": sysconfig.get_config_var("SOABI"), "monotonic_clock": time.get_clock_info("monotonic").implementation,
            "pid": os.getpid()}


def check_text(value, expected):
    received = value.value
    wire.require(value.tag == "character_string" and received == expected, "wrong text value or round identity")
    return hashlib.sha256(received.encode("ascii")).hexdigest()


async def round_trip(rb, client, events, target, router_mac, peer, seed, number, row):
    p = rb.PropertyIdentifier
    ai = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
    device = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 10)
    texts = {name: wire.text_value(seed, number, name) for name in ("rp", "rpm", "cov")}
    deadlines = row["timestamps_ns"]
    checked = row["value_sha256"]

    async def read_one(pending):
        checked["rp"] = check_text(await pending, texts["rp"])
        deadlines["rp_validated"] = time.monotonic_ns()

    async def read_many(pending):
        obj, = await pending
        wire.require(obj["object_id"] == device, "RPM object identity")
        value, count, error = obj["results"]
        wire.require((value["property_id"], value["array_index"], value["error"]) == (p.DESCRIPTION, None, None),
                     "RPM value selector or error")
        checked["rpm"] = check_text(value["value"], texts["rpm"])
        wire.require((count["property_id"], count["array_index"], count["error"], count["value"].tag,
                      count["value"].value) == (p.OBJECT_LIST, 0, None, "unsigned", 7 + number % 2),
                     "RPM Object_List count or selector")
        wire.require((error["property_id"], error["array_index"], error["value"], error["error"]) ==
                     (p.PRESENT_VALUE, None, None, (rb.ErrorClass.PROPERTY, rb.ErrorCode.UNKNOWN_PROPERTY)),
                     "RPM error selector or code")
        deadlines["rpm_validated"] = time.monotonic_ns()

    async def observe():
        event = await anext(events)
        wire.require((event.kind, event.delivery, event.subscriber_process_identifier, event.monitored_object_identifier,
                      event.initiating_device_identifier, event.source_network, event.source_address, event.source_mac) ==
                     ("object", "confirmed", wire.PROCESS, ai, rb.ObjectIdentifier(rb.ObjectType.DEVICE, wire.DEVICE_INSTANCE),
                      wire.DNET, wire.STATION, router_mac), "COV registration or source identity")
        value, = event.values
        wire.require((value["property_id"], value["array_index"]) == (p.DESCRIPTION, None), "COV selector")
        checked["cov"] = check_text(value["value"], texts["cov"])
        deadlines["cov_validated"] = time.monotonic_ns()

    async def read_peer():
        ledger = json.loads(await peer.stdout.readline())
        row["peer"] = ledger
        wire.require("error" not in ledger, f"peer failed: {ledger.get('error')}")
        wire.require(ledger["round"] == number, "peer round differs")
        deadlines["cov_first_send"] = ledger["cov_send_ns"]

    deadlines["rp_submit"] = time.monotonic_ns()
    rp = client.read_property(target, ai, p.DESCRIPTION)
    deadlines["rpm_submit"] = time.monotonic_ns()
    rpm = client.read_property_multiple(target, [(device, [(p.DESCRIPTION, None), (p.OBJECT_LIST, 0), (p.PRESENT_VALUE, None)])])
    tasks = [asyncio.create_task(operation) for operation in (read_one(rp), read_many(rpm), observe(), read_peer())]
    try:
        await asyncio.wait_for(asyncio.gather(*tasks), 45)
    finally:
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    row["latency_ns"] = {name: deadlines[name + "_validated"] - deadlines[start]
                         for name, start in (("rp", "rp_submit"), ("rpm", "rpm_submit"), ("cov", "cov_first_send"))}
    wire.require(min(row["latency_ns"].values()) >= 0, "monotonic clocks disagree")


def process_metrics(first, last, rounds, elapsed):
    user, system = last.ru_utime - first.ru_utime, last.ru_stime - first.ru_stime
    return {"elapsed_ns": elapsed, "user_seconds": user, "system_seconds": system,
            "cpu_seconds_per_completed_round": (user + system) / rounds if rounds else None,
            "process_peak_rss": last.ru_maxrss, "peak_rss_unit": "bytes" if sys.platform == "darwin" else "KiB",
            "peak_rss_at_measurement_start": first.ru_maxrss}


async def sample(args, report):
    import rusty_bacnet as rb

    endpoint = rb.BipEndpoint(device_instance=100, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                              segmentation=rb.Segmentation.SEGMENTED_RECEIVE, apdu_retries=0, apdu_timeout_ms=30000)
    peer = lease = None
    await endpoint.start()
    try:
        port = (await endpoint.local_address()).rsplit(":", 1)[1]
        peer = await asyncio.create_subprocess_exec(sys.executable, wire.__file__, port, str(args.seed),
                                                    str(args.warmup + args.rounds), stdout=asyncio.subprocess.PIPE)
        ready = json.loads(await asyncio.wait_for(peer.stdout.readline(), 30))
        router = "127.0.0.1:" + str(ready["port"])
        router_mac = socket.inet_aton("127.0.0.1") + ready["port"].to_bytes(2, "big")
        target = rb.RoutedTarget(wire.DNET, wire.STATION, router=router)
        client = await endpoint.client(confirmed_cov_subscription_limit=1)
        events = await client.cov_notifications()
        lease = await client.expect_confirmed_cov(target, wire.PROCESS, rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1),
                                                   initiating_device_instance=wire.DEVICE_INSTANCE)
        report["peer_pid"] = peer.pid
        for phase, count in (("warmup", args.warmup), ("measured", args.rounds)):
            report["phase"] = phase
            start_cpu = resource.getrusage(resource.RUSAGE_SELF)
            start = time.monotonic_ns()
            try:
                for _ in range(count):
                    number = len(report["rounds"])
                    row = {"round": number, "phase": phase, "complete": False, "timestamps_ns": {}, "value_sha256": {}}
                    report["rounds"].append(row)
                    await round_trip(rb, client, events, target, router_mac, peer, args.seed, number, row)
                    row["complete"] = True
            finally:
                elapsed = time.monotonic_ns() - start
                end_cpu = resource.getrusage(resource.RUSAGE_SELF)
                completed = sum(row["complete"] for row in report["rounds"] if row["phase"] == phase)
                report["process"][phase] = process_metrics(start_cpu, end_cpu, completed, elapsed)
        report["phase"] = "finish"
        wire.require(await asyncio.wait_for(peer.wait(), 30) == 0, "peer failed")
        status = await endpoint.status()
        report["final"] = {"cov_missed": events.missed, "active_leases": status["active_leases"],
                           "ready": status["transport_health"]["ready"]}
        wire.require(report["final"] == {"cov_missed": 0, "active_leases": 0, "ready": True}, "endpoint finish state")
    finally:
        if peer is not None and peer.returncode is None:
            try:
                peer.kill()
            except ProcessLookupError:
                pass
            await peer.wait()
        if lease is not None:
            lease.close()
        await endpoint.close()


def summarize(report):
    rows = [row for row in report["rounds"] if row["phase"] == "measured"]
    complete = [row for row in rows if row["complete"]]
    summary = {"measured_completed": len(complete), "measured_failed": len(rows) - len(complete),
               "measured_unstarted": report["parameters"]["rounds"] - len(rows), "latency_ns": {}}
    for name in ("rp", "rpm", "cov"):
        values = sorted(row["latency_ns"][name] for row in complete)
        if values:
            summary["latency_ns"][name] = {"n": len(values), "max": values[-1], **{
                f"p{p}": values[math.ceil(p * len(values) / 100) - 1] for p in (50, 95, 99)}}
    counts = Counter()
    for row in rows:
        counts.update(row.get("peer", {}).get("counts", {}))
    summary["peer_counts"] = dict(counts)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", type=Path, required=True)
    parser.add_argument("--source-sha", required=True, help="operator-declared full source commit of the wheel")
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--rounds", type=int, default=50)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.warmup < 0 or args.rounds < 1 or not re.fullmatch(r"[0-9a-f]{40}", args.source_sha):
        parser.error("use nonnegative warmup, positive rounds and a full lowercase source SHA")
    report = {"success": False, "phase": "provenance", "started_utc": datetime.now(timezone.utc).isoformat(),
              "wire_scope": "scripted_peer", "environment": environment(),
              "source_sha_declared": args.source_sha, "parameters": {
                  "seed": args.seed, "warmup": args.warmup, "rounds": args.rounds,
                  "text_bytes": wire.TEXT_BYTES, "segment_bytes": wire.CHUNK, "window": wire.WINDOW},
              "rounds": [], "process": {}}
    try:
        wire.require(sys.platform.startswith("linux") or sys.platform == "darwin", "requires Linux or macOS")
        artifacts = report["artifacts"] = provenance(args.wheel)
        wire.require(artifacts["native_sha256"] == artifacts["wheel_member_sha256"],
                     "installed native extension differs from wheel member")
        report["phase"] = "setup"
        asyncio.run(sample(args, report))
        report["phase"] = "complete"
        report["success"] = True
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
    report["summary"] = summarize(report)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["success"] else 1


if __name__ == "__main__":
    sys.exit(main())
