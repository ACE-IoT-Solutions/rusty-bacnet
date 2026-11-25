#!/usr/bin/env python3
"""Build the ACE source archive once with the release-pinned maturin and prune its lock offline."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from check_artifacts import pep440
from repair_sdist import repair


def build(version: str, output: Path) -> Path:
    """Use the checkout's commit timestamp for the stock archive and its repair."""
    root = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
    pins = (root / ".github/ci-pins.env").read_text().splitlines()
    version_pin, = (line.split("=", 1)[1] for line in pins if line.startswith("MATURIN_VERSION="))
    actual = subprocess.check_output(["maturin", "--version"], text=True).strip()
    if actual != f"maturin {version_pin}":
        raise ValueError(f"expected maturin {version_pin}, found {actual}")
    epoch = subprocess.check_output(["git", "log", "-1", "--format=%ct"], cwd=root, text=True).strip()
    # Maturin/repair both read the epoch. A caller's previous value cannot
    # change a tagged release's timestamp.
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output) as tmp:
        subprocess.run(["maturin", "sdist", "-m", "crates/rusty-bacnet/Cargo.toml",
                        "--out", tmp], check=True, cwd=root, env={**os.environ, "SOURCE_DATE_EPOCH": epoch, "CARGO_NET_OFFLINE": "true"})
        archive = Path(tmp) / f"ace_rusty_bacnet-{pep440(version)}.tar.gz"
        if list(Path(tmp).glob("*.tar.gz")) != [archive]:
            raise ValueError(f"expected a single {archive.name} source archive")
        repair(archive, epoch=int(epoch))
        final = output / archive.name
        archive.replace(final)
    return final


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(build(args.version, args.output))


if __name__ == "__main__":
    main()
