#!/usr/bin/env python3
"""Repair the Cargo.lock that maturin copies into a pruned source workspace.

Only the archive's root Cargo.lock changes. Resolve offline, prove the result
works with --locked, then replace the archive atomically. SOURCE_DATE_EPOCH
pins the gzip timestamp, as it does for the original maturin sdist.
"""

import argparse
import copy
import gzip
import io
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile
import tomllib


def repair(archive: Path, *, epoch: int | None = None, env: dict[str, str] | None = None) -> None:
    """Prune a source archive's lockfile without updating registry packages."""
    archive = archive.resolve()
    with tempfile.TemporaryDirectory(prefix="bacnet-sdist-") as temporary:
        source = Path(temporary)
        with tarfile.open(archive, "r:gz") as original:
            members = original.getmembers()
            roots = set()
            names = set()
            for member in members:
                path = PurePosixPath(member.name)
                if path.is_absolute() or ".." in path.parts or not path.parts:
                    raise ValueError(f"unsafe source archive path: {member.name}")
                if member.name in names:
                    raise ValueError(f"duplicate source archive path: {member.name}")
                names.add(member.name)
                # Validate every member before writing even into the temporary
                # directory; data_filter rejects special files and link escapes.
                tarfile.data_filter(member, source)
                roots.add(path.parts[0])
            if len(roots) != 1:
                raise ValueError("source archive must contain exactly one root directory")
            root = roots.pop()
            lock_name = f"{root}/Cargo.lock"
            locks = [member for member in members if member.name == lock_name]
            if len(locks) != 1 or not locks[0].isfile():
                raise ValueError("source archive must contain one regular root Cargo.lock")
            original.extractall(source, filter="data")
            workspace = source / root
            before = tomllib.loads((workspace / "Cargo.lock").read_text())
            command = ["cargo", "metadata", "--format-version", "1", "--offline"]
            subprocess.run(command, cwd=workspace, env=env, check=True, stdout=subprocess.DEVNULL)
            subprocess.run([*command, "--locked"], cwd=workspace, env=env, check=True,
                           stdout=subprocess.DEVNULL)
            lock = (workspace / "Cargo.lock").read_bytes()
            identity = lambda package: (package["name"], package["version"],
                                        package.get("source"), package.get("checksum"))
            allowed = {identity(package) for package in before["package"]}
            after = tomllib.loads(lock.decode())
            if any(identity(package) not in allowed for package in after["package"]):
                raise ValueError("source lock repair would introduce a new package or version")
            if epoch is None:
                epoch = int(os.environ.get("SOURCE_DATE_EPOCH", locks[0].mtime))
            # Keep all original members and metadata; only Cargo.lock's payload
            # and size change. In particular, notices and package data survive.
            pending = None
            try:
                with tempfile.NamedTemporaryFile(dir=archive.parent, delete=False) as output:
                    pending = Path(output.name)
                    with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch) as zipped:
                        with tarfile.open(fileobj=zipped, mode="w", format=tarfile.PAX_FORMAT) as result:
                            for member in members:
                                info = copy.copy(member)
                                if member.name == lock_name:
                                    info.size = len(lock)
                                    result.addfile(info, io.BytesIO(lock))
                                else:
                                    result.addfile(info, original.extractfile(member) if member.isfile() else None)
                pending.chmod(archive.stat().st_mode & 0o777)
                pending.replace(archive)
            finally:
                if pending is not None:
                    pending.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    repair(args.archive)
    print(f"repaired and verified locked offline metadata: {args.archive}")


if __name__ == "__main__":
    main()
