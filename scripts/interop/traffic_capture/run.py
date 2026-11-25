#!/usr/bin/env python3
"""Create one rootless, loopback-only capture namespace and copy out its evidence."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import traceback

HERE = Path(__file__).resolve().parent
FILES = ("run.py", "worker.py", "oracle.py", "pcap.py", "controls.py", "Dockerfile", "README.md")

# This program arrives via argv. Only copies of verified code and wheel bytes
# enter the Python import/install paths, including when Podman is remote.
BOOTSTRAP = """
import hashlib, json, os, pathlib, subprocess, sys
root = pathlib.Path("/tmp/verified")
out = pathlib.Path("/tmp/traffic-output")
root.mkdir()
out.mkdir()
expected = json.loads(sys.argv[1])
wheel = pathlib.Path(sys.argv[2])
actual = {}
for name in expected:
    original = wheel if name == wheel.name else pathlib.Path("/input") / name
    data = original.read_bytes()
    actual[name] = hashlib.sha256(data).hexdigest()
    (root / name).write_bytes(data)
(out/"inputs.json").write_text(json.dumps(actual))
if expected != actual:
    raise SystemExit("input hashes differ before fixture startup")
verified_wheel = root / wheel.name
subprocess.run([sys.executable, "-I", "-m", "venv", "/tmp/sdk"], check=True)
python = "/tmp/sdk/bin/python"
subprocess.run([python, "-I", "-m", "pip", "--disable-pip-version-check", "--no-cache-dir", "install",
                "--no-index", "--no-deps", str(verified_wheel)], check=True)
os.execv(python, [python, "-I", "-c",
                  "import sys; sys.path.insert(0, '/tmp/verified'); from worker import main; sys.exit(main())",
                  sys.argv[3], str(verified_wheel), *sys.argv[4:]])
"""


def run(args, report):
    cli = ["podman"] + (["--connection", args.connection] if args.connection else [])

    def podman(*cmd, timeout=60, check=True):
        result = subprocess.run(cli + list(cmd), text=True, capture_output=True, timeout=timeout)
        if check and result.returncode:
            raise RuntimeError(f"podman {cmd[0]}: {result.stderr.strip()}")
        return result

    info = json.loads(podman("info", "--format", "json").stdout)
    host = info["host"]
    if not host["security"]["rootless"] or host["os"] != "linux":
        raise ValueError("capture requires rootless Linux Podman")
    report["engine"] = {"connection": args.connection, "arch": host["arch"], "kernel": host["kernel"],
                        "rootless": True, "version": info["version"]["Version"]}
    image = podman("image", "inspect", "--format", "{{.Id}}", args.image).stdout.strip()
    report["image"] = image
    expected = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in FILES}
    wheel = args.wheel.resolve(strict=True)
    expected[wheel.name] = hashlib.sha256(wheel.read_bytes()).hexdigest()
    report["expected_inputs"] = expected
    cid = podman("create", "--network", "none", "--cap-drop", "ALL", "--cap-add", "NET_RAW",
                 "-v", f"{HERE}:/input:ro", "-v", f"{wheel}:/wheel/{wheel.name}:ro",
                 "--entrypoint", "python3.13", image, "-I", "-S", "-c", BOOTSTRAP, json.dumps(expected),
                 "/wheel/" + wheel.name, args.owner, str(args.idle_seconds),
                 *(["--inject"] if args.inject_unexpected else [])).stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{64}", cid):
        raise RuntimeError("invalid container ID from podman create")
    report["container_id"] = cid
    try:
        result = podman("start", "--attach", cid, timeout=200, check=False)
        report["container_exit"] = result.returncode
        (args.output / "container.log").write_text(result.stdout + result.stderr)
        podman("cp", cid + ":/tmp/traffic-output/.", str(args.output.resolve()))
        mounted = json.loads((args.output / "inputs.json").read_text())
        if mounted != expected:
            raise ValueError("mounted wheel or source hash mismatch")
        inner = json.loads((args.output / "receipt.json").read_text())
        report["worker_errors"] = {k: inner[k] for k in ("error", "capture_error", "audit_error", "cleanup_errors")
                                   if inner.get(k)}
        pcap = args.output / "loopback.pcap"
        # Audit the copied bytes again on the host before accepting them.
        if not pcap.exists() or hashlib.sha256(pcap.read_bytes()).hexdigest() != inner.get("pcap_sha256"):
            raise ValueError(f"missing complete verified capture: {inner}")
        from pcap import audit
        verified = audit(pcap.read_bytes(), args.owner, inner["nonce"], args.idle_seconds)
        if result.returncode or not inner["success"] or verified != inner["audit"]:
            raise ValueError(f"capture, SDK operations, or container audit failed: {report['worker_errors']}")
        from controls import rejected_mutations
        report["offline_controls"] = rejected_mutations(pcap.read_bytes(), args.owner, inner["nonce"], args.idle_seconds)
    finally:
        if not (args.output / "receipt.json").exists():
            try:
                podman("cp", cid + ":/tmp/traffic-output/.", str(args.output.resolve()))
            except Exception as error:
                report["cleanup_errors"].append(f"retaining partial container evidence: {error}")
        try:
            podman("rm", "--force", cid)
            report["removed"] = True
        except Exception as error:
            report["cleanup_errors"].append(str(error))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--owner", choices=["shared", "standalone"], required=True)
    p.add_argument("--wheel", type=Path, required=True)
    p.add_argument("--image", required=True, help="locally available capture image")
    p.add_argument("--connection")
    p.add_argument("--source", required=True, help="operator-declared wheel source SHA")
    p.add_argument("--idle-seconds", type=float, default=2.0)
    p.add_argument("--inject-unexpected", action="store_true")
    p.add_argument("--output", type=Path, required=True, help="new evidence directory")
    args = p.parse_args()
    if not math.isfinite(args.idle_seconds) or not 0 < args.idle_seconds <= 30:
        p.error("--idle-seconds must be in (0, 30]")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.source):
        p.error("--source must be a full Git SHA")
    args.output.mkdir(parents=True, exist_ok=False)
    report = {"operator_declared_source": args.source, "owner": args.owner, "cleanup_errors": [],
              "inject_unexpected": args.inject_unexpected}
    try:
        run(args, report)
    except (Exception, KeyboardInterrupt):
        report["error"] = traceback.format_exc()
    report["success"] = not report.get("error") and not report["cleanup_errors"]
    (args.output / "run.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"success": report["success"], "evidence": str(args.output)}))
    return int(not report["success"])


if __name__ == "__main__":
    raise SystemExit(main())
