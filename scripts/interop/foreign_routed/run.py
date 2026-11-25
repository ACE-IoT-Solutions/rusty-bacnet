#!/usr/bin/env python3
"""Run the foreign-device/routed-peer probe in a fresh rootless Podman namespace."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import traceback

HERE = Path(__file__).resolve().parent
# Pass bootstrap and expected hashes over the engine API. Remote mounts may
# resolve to different files. Install and execute only the verified copies.
BOOT = """
import hashlib, json, os, pathlib, subprocess, sys
root = pathlib.Path('/tmp/verified')
root.mkdir()
wheel = pathlib.Path(sys.argv[1])
actual = {}
for name, digest in json.loads(sys.argv[2]).items():
    path = wheel if name == wheel.name else pathlib.Path('/fixture') / name
    data = path.read_bytes()
    actual[name] = hashlib.sha256(data).hexdigest()
    if actual[name] != digest:
        raise ValueError('mounted input hash mismatch: ' + name)
    (root / name).write_bytes(data)
(root / 'inputs.json').write_text(json.dumps(actual))
subprocess.run([sys.executable, '-I', '-m', 'venv', '/tmp/sdk'], check=True)
exe = '/tmp/sdk/bin/python'
subprocess.run([exe, '-I', '-m', 'pip', '--disable-pip-version-check', 'install',
                '--no-index', '--no-deps', str(root / wheel.name)], check=True)
os.execv(exe, [exe, '-I', str(root / 'probe.py'), str(root / wheel.name), sys.argv[3]])
"""


def qualify(args, report):
    cli = ["podman"] + (["--connection", args.connection] if args.connection else [])

    def podman(*parts, check=True, timeout=60):
        proc = subprocess.run(cli + list(parts), capture_output=True, text=True, timeout=timeout)
        if check and proc.returncode:
            raise RuntimeError(f"podman {parts[0]}: {proc.stderr.strip()}")
        return proc

    host = json.loads(podman("info", "--format", "json").stdout)["host"]
    if host["os"] != "linux" or not host["security"]["rootless"]:
        raise ValueError("requires rootless Linux Podman")
    report["engine"] = dict(rootless=True, arch=host["arch"], kernel=host["kernel"],
                            connection=args.connection)
    image = podman("image", "inspect", "--format", "{{.Id}}", args.image).stdout.strip()
    report["image"] = image
    wheel = args.wheel.resolve(strict=True)
    paths = [HERE / n for n in ("probe.py", "run.py", "README.md")] + [wheel]
    expected = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    report["expected_inputs"] = expected
    cid = podman("create", "--network", "none", "--cap-drop", "ALL", "--entrypoint", "python3.13",
                 "-v", f"{HERE}:/fixture:ro", "-v", f"{wheel}:/wheel/{wheel.name}:ro", image,
                 "-I", "-S", "-c", BOOT, "/wheel/" + wheel.name, json.dumps(expected), args.profile).stdout.strip()
    if not re.fullmatch("[a-f0-9]{64}", cid):
        raise ValueError("engine returned invalid container ID")
    report["container"] = cid
    try:
        proc = podman("start", "--attach", cid, check=False, timeout=150)
        (args.output / "container.log").write_text(proc.stdout + proc.stderr)
        report["container_exit"] = proc.returncode
        podman("cp", cid + ":/tmp/receipt.json", str(args.output / "receipt.json"))
        receipt = json.loads((args.output / "receipt.json").read_text())
        if (proc.returncode or not receipt["success"] or receipt["mounted_inputs"] != expected
                or receipt["profile"] != args.profile):
            raise ValueError(f"probe failed; see receipt.json and container.log in {args.output}")
    finally:
        if not (args.output / "receipt.json").exists():
            try:
                podman("cp", cid + ":/tmp/receipt.json", str(args.output / "receipt.json"))
            except Exception as error:
                report["cleanup_errors"].append(f"retaining partial receipt: {error}")
        try:
            podman("rm", "--force", cid)
            report["removed"] = True
        except Exception as error:
            report["cleanup_errors"].append(str(error))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--source", required=True, help="operator-declared source SHA of the wheel")
    parser.add_argument("--profile", choices=["unknown", "configured7"], default="unknown",
                        help="default unknown local network number or registered Network Port 7")
    parser.add_argument("--connection")
    parser.add_argument("--output", required=True, type=Path, help="new receipt directory")
    args = parser.parse_args()
    if not re.fullmatch("[a-f0-9]{40}", args.source):
        parser.error("--source requires a full source commit SHA")
    args.output.mkdir(parents=True, exist_ok=False)
    report = {"operator_declared_source": args.source, "profile": args.profile, "cleanup_errors": []}
    try:
        qualify(args, report)
    except (Exception, KeyboardInterrupt):
        report["error"] = traceback.format_exc()
    report["success"] = not report.get("error") and not report["cleanup_errors"]
    (args.output / "run.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"success": report["success"], "evidence": str(args.output)}))
    return int(not report["success"])


if __name__ == "__main__":
    sys.exit(main())
