#!/usr/bin/env python3
"""Qualify two Rusty endpoints sharing a B/IP port from separate rootless host-network containers."""

import argparse
import asyncio
import hashlib
import ipaddress
import json
from pathlib import Path
import re
import signal
import subprocess
import sys

FIXTURE = Path(__file__).resolve().parent


# Pass this code through argv: a remote mount must be verified before any
# mounted Python file, pip installer or SDK extension can execute.
VERIFY_INPUTS = """
import hashlib, json, os, pathlib, sys
expected = json.loads(sys.argv[1])
actual = {"fixture_sha256": {}, "wheel_sha256": None}
try:
    for name in expected["fixture_sha256"]:
        with (pathlib.Path("/probe") / name).open("rb") as stream:
            actual["fixture_sha256"][name] = hashlib.file_digest(stream, "sha256").hexdigest()
    with open(sys.argv[2], "rb") as stream:
        actual["wheel_sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
except OSError as error:
    actual["error"] = str(error)
print(json.dumps(actual), flush=True)
if actual != expected:
    sys.exit(1)
os.execv(sys.argv[3], sys.argv[3:])
"""


def check(condition, message):
    if not condition:
        raise ValueError(message)


class Engine:
    def __init__(self, args, report):
        self.command = ["podman"] + (["--connection", args.connection] if args.connection else [])
        self.wheel = args.wheel.resolve()
        self.report = report
        self.attached = {}
        host_info = json.loads(self.run("info", "--format", "json"))
        host = host_info["host"]
        report["engine"] = {"connection": args.connection, "hostname": host["hostname"],
                            "os": host["os"], "arch": host["arch"], "kernel": host["kernel"],
                            "rootless": host["security"]["rootless"], "id_mappings": host["idMappings"],
                            "version": host_info["version"]["Version"]}
        check(host["security"]["rootless"] and host["os"] == "linux", "this probe requires rootless Linux Podman")
        self.image = self.run("image", "inspect", "--format", "{{.Id}}", args.image)
        report["image"] = self.image

    def run(self, *args):
        result = subprocess.run([*self.command, *args], capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError(f"podman {args[0]} failed: {result.stderr.strip()}")
        return result.stdout.strip()

    async def start(self, role, uid, command):
        mounted_wheel = "/wheel/" + self.wheel.name
        mounts = ["-v", f"{FIXTURE}:/probe:ro", "-v", f"{self.wheel}:{mounted_wheel}:ro"]
        expected = {key: self.report[key] for key in ("fixture_sha256", "wheel_sha256")}
        container = self.run("create", "--interactive", "--network", "host", "--user", f"{uid}:{uid}",
                             *mounts, "--entrypoint", "/usr/bin/python3.13", self.image,
                             "-I", "-S", "-c", VERIFY_INPUTS, json.dumps(expected), mounted_wheel, *command)
        self.report["containers"][role] = {"id": container, "uid_requested": uid}
        mode = self.run("inspect", "--format", "{{.HostConfig.NetworkMode}}", container)
        self.report["containers"][role]["network_mode"] = mode
        check(mode == "host", f"{role} does not use the engine host network")
        proc = await asyncio.create_subprocess_exec(*self.command, "start", "-ai", container, stdin=asyncio.subprocess.PIPE,
                                                   stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        self.attached[role] = proc, asyncio.create_task(proc.stderr.read())
        mounted = await self.receive(role)
        self.report["containers"][role]["mounted"] = mounted
        check(mounted == expected, f"{role} mounted inputs differ from the runner's fixture or wheel")
        first = await self.receive(role)
        self.report["containers"][role]["startup"] = first
        identity = first["identity"]
        maps = self.report["engine"]["id_mappings"]["uidmap"]
        check(identity["uid"] == uid and identity["uid_map"] ==
              [[r["container_id"], r["host_id"], r["size"]] for r in maps], f"{role} effective UID or map mismatch")
        mapped, = [r["host_id"] + uid - r["container_id"] for r in maps if r["container_id"] <= uid <
                   r["container_id"] + r["size"]]
        check(identity["mapped_uid"] == mapped, f"{role} host UID mismatch")
        if role != "controller":
            check(identity["net_namespace"] == self.report["containers"]["controller"]["startup"]["identity"]["net_namespace"],
                  f"{role} has a different network namespace from the host-network controller")
        return first

    async def receive(self, role):
        proc, stderr = self.attached[role]
        line = await asyncio.wait_for(proc.stdout.readline(), 60)
        if not line:
            proc.stdin.close()
            await asyncio.wait_for(proc.wait(), 15)
            raise RuntimeError(f"{role} exited {proc.returncode}: {(await stderr).decode().strip()}")
        return json.loads(line)

    async def ask(self, role, command):
        proc, _ = self.attached[role]
        proc.stdin.write(json.dumps(command).encode() + b"\n")
        await proc.stdin.drain()
        answer = await self.receive(role)
        self.report["transcript"].append({"role": role, "command": command, "reply": answer})
        return answer

    async def wait_for_exit(self, role):
        proc, stderr = self.attached[role]
        proc.stdin.close()
        status = await asyncio.wait_for(proc.wait(), 20)
        check(status == 0, f"{role} exited {status}: {(await stderr).decode().strip()}")

    async def close(self, role):
        check((await self.ask(role, {"op": "close"})).get("closed"), f"{role} did not close")
        await self.wait_for_exit(role)

    async def cleanup(self):
        failures = []
        for role, (proc, _) in self.attached.items():
            try:
                proc.stdin.close()
            except Exception as error:
                failures.append(f"{role} stdin: {error}")
        for role, record in reversed(list(self.report["containers"].items())):
            try:
                self.run("rm", "--force", record["id"])
                record["removed"] = True
            except Exception as error:
                failures.append(f"{role}: {error}")
        for role, (proc, stderr) in self.attached.items():
            try:
                await asyncio.wait_for(proc.wait(), 15)
            except Exception as error:
                failures.append(f"{role} attachment: {type(error).__name__}: {error}")
                if proc.returncode is None:
                    try:
                        proc.kill()
                        await asyncio.wait_for(proc.wait(), 5)
                    except ProcessLookupError:
                        pass
                    except Exception as stop_error:
                        failures.append(f"{role} attachment stop: {stop_error}")
            try:
                await asyncio.wait_for(stderr, 15)
            except (Exception, asyncio.CancelledError) as error:
                failures.append(f"{role} stderr: {type(error).__name__}: {error}")
        return failures


async def worker(engine, role, config, index, name, uid, *, must_start=True):
    entry = config["endpoints"][index]
    wheel = "/wheel/" + engine.wheel.name
    install = ('set -eu\npython3.13 -m venv /tmp/sdk\n'
               '/tmp/sdk/bin/python -m pip --no-cache-dir --disable-pip-version-check install --no-index --no-deps "$1" >&2\n'
               'exec /tmp/sdk/bin/python /probe/worker.py "$@"')
    record = await engine.start(role, uid, ["/bin/sh", "-c", install, "start", wheel, entry["cidr"],
                                            str(config["port"]), str(entry["instance"]), name])
    if must_start:
        addr = f'{ipaddress.IPv4Interface(entry["cidr"]).ip}:{config["port"]}'
        check(record["started"] and record["address"] == addr, f"{role} startup or address failed: {record}")
        health = record["health"]
        check(health["ready"] and health["primary_receive_active"] and 0 < health["broadcast_listeners_required"] ==
              health["broadcast_listeners_active"], f"{role} broadcast listener health failed: {health}")
    else:
        check(not record["started"] and record["error"]["errno_name"] == "EADDRINUSE"
              and record["error"]["type"] == "BacnetTransportError", f"{role} expected EADDRINUSE: {record}")
        await engine.wait_for_exit(role)
    return record


async def workflow(args, report):
    engine = Engine(args, report)
    config = report["config"]
    try:
        controller = await engine.start("controller", 0, ["/opt/qa/bin/python", "/probe/controller.py", json.dumps(config)])
        check(controller["bacpypes3"] == "0.0.106", "controller image must provide BACpypes3 0.0.106")
        preflight = await engine.ask("controller", {"op": "preflight"})
        check(preflight["ok"], f"exclusive socket preflight refused; no SDK worker was started: {preflight}")

        async def name(index, expected):
            nonlocal invoke
            invoke += 1
            answer = await engine.ask("controller", {"op": "name", "index": index, "invoke": invoke})
            check(answer["ok"] and answer["name"] == expected, f"unexpected Device name: {answer}")

        invoke = 0
        first = await worker(engine, "first", config, 0, "hostnet-first", 10001)
        second = await worker(engine, "second", config, 1, "hostnet-second", 10002)
        check(first["identity"]["mapped_uid"] != second["identity"]["mapped_uid"], "owner kernel UIDs coincide")
        occupied = await engine.ask("controller", {"op": "preflight"})
        check(not occupied["ok"] and occupied["error"]["errno_name"] == "EADDRINUSE", "occupied preflight accepted")
        await worker(engine, "contender", config, 0, "hostnet-contender", 10003, must_start=False)
        check((await engine.ask("controller", {"op": "discover"}))["ok"], "targeted broadcast failed")
        await name(0, "hostnet-first")
        await name(1, "hostnet-second")
        await engine.close("first")
        # A replacement of the first Device tries the second owner's live address.
        replacement = {**config, "endpoints": [{**config["endpoints"][0], "cidr": config["endpoints"][1]["cidr"]}]}
        await worker(engine, "failed_replacement", replacement, 0, "hostnet-replacement", 10001, must_start=False)
        await name(1, "hostnet-second")
        await worker(engine, "restored", config, 0, "hostnet-restored", 10001)
        await name(0, "hostnet-restored")
        await name(1, "hostnet-second")
        await engine.close("restored")
        await engine.close("second")
        check((await engine.ask("controller", {"op": "preflight"}))["ok"], "socket preflight after close failed")
        await engine.close("controller")
    finally:
        try:
            report["cleanup_errors"] = await engine.cleanup()
        except Exception as error:
            report["cleanup_errors"] = [f"{type(error).__name__}: {error}"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--connection")
    parser.add_argument("--image", default="localhost/rusty-bacnet-downstream-qa:rust1.99-py313")
    parser.add_argument("--cidr-a")
    parser.add_argument("--cidr-b")
    parser.add_argument("--controller-cidr")
    parser.add_argument("--device-a", type=int)
    parser.add_argument("--device-b", type=int)
    parser.add_argument("--port", type=int, default=47808)
    args = parser.parse_args()
    interfaces = [ipaddress.IPv4Interface(value or default) for value, default in
                  zip((args.cidr_a, args.cidr_b, args.controller_cidr), ("127.0.0.1/8", "127.0.0.2/8", "127.0.0.10/8"))]
    if any(not iface.ip.is_loopback for iface in interfaces) and None in (
            args.cidr_a, args.cidr_b, args.controller_cidr, args.device_a, args.device_b):
        parser.error("supply three preassigned CIDRs and two unused Device IDs for a non-loopback run")
    if args.device_a is None:
        args.device_a = 7101
    if args.device_b is None:
        args.device_b = 7102
    if (len({i.ip for i in interfaces}) != 3 or not 1 <= args.port <= 65535 or args.device_a == args.device_b
            or not all(0 <= i < 4194303 for i in (args.device_a, args.device_b))):
        parser.error("supply distinct addresses, two distinct concrete Device IDs and a valid UDP port")
    if any(i.network.prefixlen > 30 or i.ip.is_multicast or i.ip.is_unspecified or
           i.ip in (i.network.broadcast_address, i.network.network_address) for i in interfaces):
        parser.error("each CIDR requires a specific unicast address and a directed broadcast")
    if not re.fullmatch("[a-f0-9]{40}", args.source_sha):
        parser.error("supply a full lowercase source SHA")
    config = {"controller": str(interfaces[2]), "port": args.port, "endpoints": [
        {"cidr": str(iface), "instance": instance} for iface, instance in zip(interfaces, (args.device_a, args.device_b))]}
    report = {"success": False, "config": config, "source_sha_declared": args.source_sha, "containers": {}, "transcript": [],
              "wheel": args.wheel.name, "fixture_sha256": {
                  path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(FIXTURE.glob("*.py"))}}
    async def run():
        loop = asyncio.get_running_loop()
        loop.add_signal_handler(signal.SIGTERM, asyncio.current_task().cancel)
        await workflow(args, report)
    try:
        with args.wheel.open("rb") as stream:
            report["wheel_sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
        asyncio.run(run())
        check(not report["cleanup_errors"], f"cleanup failed: {report['cleanup_errors']}")
        report["success"] = True
    except (Exception, KeyboardInterrupt, asyncio.CancelledError) as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    if not report["success"]:
        print(report["failure"], file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
