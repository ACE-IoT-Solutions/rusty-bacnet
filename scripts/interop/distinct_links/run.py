#!/usr/bin/env python3
"""Qualify named-interface broadcast admission on two private Linux bridges."""
import argparse
import copy
import hashlib
import ipaddress
import json
from pathlib import Path
import re
import selectors
import socket
import subprocess
import sys
import time
import traceback
import uuid

HERE = Path(__file__).resolve().parent
LABEL = "org.rusty-bacnet.distinct-links"
PHASES = ("limited", "directed", "unicast")
# Executed from argv. Remote mounts must match the operator's inputs before
# their contents enter either the SDK install path or Python's import path.
BOOT = """
import hashlib,json,pathlib,subprocess,sys
wheel=pathlib.Path(sys.argv[1])
dst=pathlib.Path('/tmp/verified')
dst.mkdir()
actual={}
for name,expected in json.loads(sys.argv[2]).items():
    path=wheel if name==wheel.name else pathlib.Path('/input')/name
    data=path.read_bytes()
    actual[name]=hashlib.sha256(data).hexdigest()
    if actual[name]!=expected:
        raise ValueError('input digest mismatch: '+name)
    (dst/name).write_bytes(data)
if sys.argv[3]=='sut':
    subprocess.run([sys.executable,'-I','-m','venv','/tmp/sdk'],check=True)
    subprocess.run(['/tmp/sdk/bin/python','-I','-m','pip','--disable-pip-version-check',
                    'install','--no-index','--no-deps',str(dst/wheel.name)],check=True,stdout=sys.stderr)
print(json.dumps(actual))
"""


def check(ok, message):
    if not ok:
        raise ValueError(message)


def witness(report, phase, step):
    """Validate the retained admission tables, source MACs, events and mismatch indices."""
    config = report["links"]
    indices = [nic["index"] for nic in report["ready"]["interfaces"]]
    check(len(set(indices)) == 2 and all(indices), "distinct ingress indices required")
    pairs = []
    for warning in step["warnings"]:
        check(warning["target"] == "bacnet_transport::bip::ingress" and warning["level"] == "WARNING",
              f"wrong warning source: {warning}")
        pairs.append((warning["fields"]["own"], warning["fields"]["arrival"]))
    a, b = [f"Some({i})" for i in indices]
    check(sorted(pairs) == sorted([(a, b), (b, a)]), f"wrong-interface warning pairs {pairs}")
    for i, link in enumerate(config):
        ids = [12000 + 100 * i + n for n in range(PHASES.index(phase) + 1)]
        sources = [(socket.inet_aton(link["peer"]) + (47900 + n).to_bytes(2, "big")).hex()
                   for n in range(len(ids))]
        check(step["tables"][i] == [[v, source] for v, source in zip(ids, sources)],
              f"foreign or missing admitted I-Am at {link['name']}")
        expected = []
        for n, (v, source) in enumerate(zip(ids, sources)):
            value = 10 * (i + 1) + n
            expected.append(dict(instance=v, source=source, value=value, raw=bytes([0x21, value]).hex()))
        check(step["events"][i] == expected, f"foreign, missing or duplicate COV at {link['name']}")


def health(rows):
    check(len(rows) == 2 and all(r["ready"] and r["primary_receive_active"]
                                and r["broadcast_listeners_required"] == r["broadcast_listeners_active"] == 2
                                for r in rows), f"both subnet and limited listeners must be active: {rows}")


def controls(report):
    cases = []
    for name in ("incorrect_ingress_index", "foreign_event", "missing_admitted_device"):
        changed = copy.deepcopy(report["closed"])
        if name == "incorrect_ingress_index":
            fields = changed["warnings"][0]["fields"]
            fields["arrival"] = fields["own"]
        elif name == "foreign_event":
            changed["events"][0].append(changed["events"][1][0])
        else:
            changed["tables"][0].pop()
        try:
            witness(report, "unicast", changed)
        except ValueError as error:
            cases.append(dict(case=name, rejection=str(error)))
        else:
            raise ValueError(f"oracle accepted control {name}")
    return cases


class Engine:
    def __init__(self, args, report):
        self.cli = ["podman"] + (["--connection", args.connection] if args.connection else [])
        self.args = args
        self.report = report
        self.token = uuid.uuid4().hex
        self.networks = {}
        self.containers = {}
        self.image = self.cmd("image", "inspect", "--format", "{{.Id}}", args.image)
        self.report["image"] = self.image
        self.proc = None
        self.stderr = None

    def cmd(self, *parts, timeout=60):
        p = subprocess.run(self.cli + list(parts), capture_output=True, text=True, timeout=timeout)
        if p.returncode:
            raise RuntimeError(f"podman {parts[0]}: {p.stderr.strip()} {p.stdout.strip()}")
        return p.stdout.strip()

    def preflight(self):
        info = json.loads(self.cmd("info", "--format", "json"))
        h = info["host"]
        check(h["security"]["rootless"] and h["os"] == "linux" and h["networkBackend"] == "netavark",
              "requires rootless Linux Netavark")
        self.report["engine"] = dict(rootless=True, backend=h["networkBackendInfo"], arch=h["arch"],
                                     kernel=h["kernel"], version=info["version"]["Version"])
        names = self.cmd("network", "ls", "--format", "{{.Name}}").splitlines()
        configured = json.loads(self.cmd("network", "inspect", *names)) if names else []
        occupied = [ipaddress.ip_network(sub["subnet"]) for net in configured for sub in net.get("subnets") or []]
        routes = self.cmd("run", "--rm", "--network", "host", "--cap-drop", "ALL", "--read-only",
                          "--entrypoint", "cat", self.image, "/proc/net/route")
        for row in routes.splitlines()[1:]:
            fields = row.split()
            mask = socket.inet_ntoa(bytes.fromhex(fields[7])[::-1])
            if mask != "0.0.0.0":
                addr = socket.inet_ntoa(bytes.fromhex(fields[1])[::-1])
                occupied.append(ipaddress.ip_network(f"{addr}/{mask}", strict=False))
        self.report["preflight_ipv4_networks"] = sorted({str(n) for n in occupied if n.version == 4})
        private = [ipaddress.ip_network(n) for n in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")]
        if self.args.subnets:
            candidates = [ipaddress.IPv4Network(n) for n in self.args.subnets]
        else:
            candidates = ipaddress.IPv4Network("10.253.0.0/16").subnets(new_prefix=28)
        chosen = []
        for n in candidates:
            check(n.prefixlen == 28 and any(n.subnet_of(p) for p in private), "choose two RFC1918 /28s")
            overlaps = any(n.overlaps(other) for other in occupied + chosen if other.version == 4)
            if self.args.subnets:
                check(not overlaps, f"subnet conflict: {n}")
            if not overlaps:
                chosen.append(n)
            if len(chosen) == 2:
                break
        check(len(chosen) == 2, "two unused subnets required")
        return [dict(subnet=str(n), sut=str(n[2]), peer=str(n[3]), name=f"veth-{'ab'[i]}", instance=3101 + i)
                for i, n in enumerate(chosen)]

    def network(self, name, cidr):
        self.cmd("network", "create", "--driver", "bridge", "--internal", "--disable-dns", "--subnet", cidr,
                 "--label", f"{LABEL}={self.token}", name)
        # Record the generated name even if inspection fails, and recheck ownership before removal.
        self.networks[name] = None
        net, = json.loads(self.cmd("network", "inspect", name))
        check(net["labels"].get(LABEL) == self.token and net["name"] == name, "network identity mismatch")
        self.networks[name] = net["id"]
        check(net["internal"] and not net["dns_enabled"] and net["driver"] == "bridge"
              and [s["subnet"] for s in net["subnets"]] == [cidr], "unexpected network topology")

    def container(self, role, links, names, inputs, wheel):
        network_args = []
        for name, ip in zip(names, links):
            network_args += ["--network", name + ":ip=" + ip]
        mounted = "/wheel/" + wheel.name
        cid = self.cmd("create", "--label", f"{LABEL}={self.token}", "--cap-drop", "ALL", *network_args,
                       "-v", f"{HERE}:/input:ro", "-v", f"{wheel}:{mounted}:ro",
                       "--entrypoint", "/bin/sleep", self.image, "infinity")
        check(re.fullmatch("[0-9a-f]{64}", cid), "invalid returned container ID")
        self.containers[role] = cid
        self.cmd("start", cid)
        actual = json.loads(self.cmd("exec", cid, "python3.13", "-I", "-S", "-c", BOOT, mounted,
                                     json.dumps(inputs), role))
        check(actual == inputs, "mounted wheel or fixture hash mismatch")
        meta, = json.loads(self.cmd("inspect", cid))
        check(meta["Id"] == cid and meta["Config"]["Labels"].get(LABEL) == self.token
              and meta["Image"].removeprefix("sha256:") == self.image.removeprefix("sha256:"), "container identity")
        networks = meta["NetworkSettings"]["Networks"]
        check(set(networks) == set(names) and all(networks[n]["IPAddress"] == ip for n, ip in zip(names, links)),
              f"unexpected attached networks for {role}")
        self.report["containers"][role] = dict(id=cid, mounted_inputs=actual, networks=networks)

    def command(self, role, config):
        exe = "/tmp/sdk/bin/python" if role == "sut" else "python3.13"
        return ["exec", "-i", self.containers[role], exe, "-I", "/tmp/verified/worker.py",
                "sut" if role == "sut" else "peer", json.dumps(config)]

    def receive(self):
        with selectors.DefaultSelector() as sel:
            sel.register(self.proc.stdout, selectors.EVENT_READ)
            check(sel.select(40), "SUT response timeout")
        line = self.proc.stdout.readline()
        check(line, "SUT exited without a receipt")
        row = json.loads(line)
        if "error" in row:
            self.report["sut_error"] = row
            raise ValueError(row["error"])
        return row

    def cleanup(self):
        errors = self.report["cleanup_errors"]
        if self.proc is not None:
            try:
                self.proc.stdin.close()
            except OSError as error:
                errors.append(str(error))
        for role, cid in reversed(list(self.containers.items())):
            try:
                meta, = json.loads(self.cmd("inspect", cid))
                check(meta["Id"] == cid and meta["Config"]["Labels"].get(LABEL) == self.token,
                      f"container ownership changed for {role}")
                self.cmd("stop", "--time", "5", cid)
                self.cmd("rm", cid)
                self.report["removed"].append(cid)
            except Exception as error:
                errors.append(f"{role}: {error}")
        for name, nid in reversed(list(self.networks.items())):
            try:
                net, = json.loads(self.cmd("network", "inspect", nid or name))
                check(net["name"] == name and net["labels"].get(LABEL) == self.token
                      and (nid is None or net["id"] == nid), "network ownership changed")
                self.cmd("network", "rm", net["id"])
                self.report["removed"].append(net["id"])
            except Exception as error:
                errors.append(f"{name}: {error}")
        if self.proc is not None:
            try:
                self.proc.wait(timeout=15)
            except Exception as error:
                errors.append(f"reaping SUT exec: {error}")
        if self.stderr is not None:
            self.stderr.close()

    def workflow(self):
        report = self.report
        links = self.preflight()
        report["links"] = links
        wheel = self.args.wheel.resolve(strict=True)
        paths = [HERE / n for n in ("worker.py", "run.py", "README.md")] + [wheel]
        inputs = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        report["expected_inputs"] = inputs
        names = [f"rusty-links-{self.token}-{i}" for i in range(2)]
        for link, name in zip(links, names):
            self.network(name, link["subnet"])
        self.container("sut", [link["sut"] for link in links], names, inputs, wheel)
        roles = ["peer-a", "peer-b"]
        for role, link, name in zip(roles, links, names):
            self.container(role, [link["peer"]], [name], inputs, wheel)
        for i, net in enumerate(json.loads(self.cmd("network", "inspect", *names))):
            check(net["name"] == names[i] and net["id"] == self.networks[names[i]]
                  and set(net["containers"]) == {self.containers["sut"], self.containers[roles[i]]},
                  "unexpected bridge membership")
            report["networks"].append(net)

        self.stderr = (self.args.output / "sut.stderr").open("w")
        self.proc = subprocess.Popen(self.cli + self.command("sut", links) + ["/tmp/verified/" + wheel.name],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        ready = self.receive()
        report["ready"] = ready
        check(ready["ready"] and not ready["warnings"], "pre-existing mismatch warning")
        health(ready["health"])
        for phase in PHASES:
            for i, role in enumerate(roles):
                obs = json.loads(self.cmd(*self.command(role, links), str(i), phase))
                report["peers"].append(obs)
                check(len({v["identity"]["namespace"] for v in report["peers"][-(i + 1):]}
                          | {ready["identity"]["namespace"]}) == i + 2, "workers share a namespace")
            self.proc.stdin.write(phase + "\n")
            self.proc.stdin.flush()
            step = self.receive()
            report["steps"].append(step)
            check(step["phase"] == phase, "wrong checkpoint")
            witness(report, phase, step)
        self.proc.stdin.write("close\n")
        self.proc.stdin.flush()
        report["closed"] = self.receive()
        final = report["closed"]
        check(final["closed"] and final["missed"] == [0, 0], "iterator close or loss")
        witness(report, "unicast", final)
        health(final["health"])
        check(self.proc.wait(timeout=20) == 0, "SUT exec failed")
        report["offline_controls"] = controls(report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--source", required=True, help="operator-declared wheel source SHA")
    parser.add_argument("--subnets", nargs=2, metavar=("A", "B"), help="two unused RFC1918 /28s")
    parser.add_argument("--connection")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not re.fullmatch("[0-9a-f]{40}", args.source):
        parser.error("expected a full Git SHA")
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(operator_declared_source=args.source, containers={}, networks=[], steps=[], peers=[],
                  cleanup_errors=[], removed=[], started=time.time())
    try:
        engine = Engine(args, report)
        try:
            engine.workflow()
        finally:
            engine.cleanup()
    except (Exception, KeyboardInterrupt):
        report["error"] = traceback.format_exc()
    report["finished"] = time.time()
    report["success"] = not report.get("error") and not report["cleanup_errors"]
    (args.output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(success=report["success"], evidence=str(args.output))))
    return int(not report["success"])


if __name__ == "__main__":
    sys.exit(main())
