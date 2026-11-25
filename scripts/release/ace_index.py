#!/usr/bin/env python3
"""Check the source and select the staged ACE distributions for the private release job."""

import argparse
from pathlib import Path
import re
import subprocess

from check_artifacts import expected_wheel_tags, pep440
from release_api import local_assets, sha256_bytes, sums_text

PLATFORMS = (
    "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "manylinux_2_17_aarch64.manylinux2014_aarch64",
    "macosx_10_12_x86_64", "macosx_11_0_arm64", "win_amd64",
)


def verify_source(version: str, commit: str) -> None:
    """Require the remote version tag, validated commit and clean tracked checkout to agree."""
    ref = f"refs/tags/v{version}"
    subprocess.run(["git", "check-ref-format", ref], check=True)
    head = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD^{commit}"], text=True).strip()
    if head != commit:
        raise ValueError(f"checked-out HEAD {head} differs from the tested commit {commit}")
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"]):
        raise ValueError("tracked release source has changed")
    remote = subprocess.check_output(["git", "ls-remote", "--exit-code", "origin", ref, ref + "^{}"], text=True)
    if advertised_tag(remote, ref, len(head)) != commit:
        raise ValueError(f"origin's {ref} does not resolve to tested commit {commit}")


def advertised_tag(response: str, ref: str, oid_length: int) -> str:
    """Read one exact ls-remote tag, using its peeled commit when present."""
    refs = {}
    peeled = ref + "^{}"
    for line in response.splitlines():
        fields = line.split("\t")
        if len(fields) != 2:
            raise ValueError("invalid remote tag advertisement")
        oid, name = fields
        if name not in (ref, peeled) or name in refs or not re.fullmatch(rf"[0-9a-f]{{{oid_length}}}", oid):
            raise ValueError("invalid or duplicate remote tag advertisement")
        refs[name] = oid
    if ref not in refs:
        raise ValueError("origin did not advertise the requested tag")
    return refs.get(peeled, refs[ref])


def private_artifacts(directory: Path, version: str, staged_sums: str) -> list[Path]:
    """Verify the full staged SHA256SUMS digest and select its exact eleven ACE files."""
    local = local_assets(directory)
    if sha256_bytes(sums_text(local).encode()) != staged_sums:
        raise ValueError("release assets differ from the GitHub draft's staged SHA256SUMS")
    stem = f"ace_rusty_bacnet-{pep440(version)}"
    names = [f"{stem}.tar.gz"]
    for python, abi, platform in sorted(expected_wheel_tags(("3.10", "3.13"), PLATFORMS)):
        names.append(f"{stem}-{python}-{abi}-{platform}.whl")
    missing = set(names) - local.keys()
    if missing:
        raise ValueError("missing private release assets: " + ", ".join(sorted(missing)))
    return [directory / name for name in names]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    source = commands.add_parser("verify-source")
    source.add_argument("version")
    source.add_argument("commit")
    assets = commands.add_parser("artifacts")
    assets.add_argument("directory", type=Path)
    assets.add_argument("version")
    assets.add_argument("staged_sums")
    args = parser.parse_args()
    if args.command == "verify-source":
        verify_source(args.version, args.commit)
    else:
        for artifact in private_artifacts(args.directory, args.version, args.staged_sums):
            print(artifact)


if __name__ == "__main__":
    main()
