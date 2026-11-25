"""Two independent source builds from an immutable synthetic Rust/Python workspace."""

import gzip
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


def pinned_builder_available():
    if not shutil.which("cargo") or not shutil.which("maturin"):
        return False
    pins = (ROOT / ".github/ci-pins.env").read_text().splitlines()
    version, = (line.split("=", 1)[1] for line in pins if line.startswith("MATURIN_VERSION="))
    installed = subprocess.run(["maturin", "--version"], capture_output=True, text=True)
    return installed.returncode == 0 and installed.stdout.strip() == "maturin " + version


@unittest.skipUnless(pinned_builder_available(), "requires the release-pinned maturin and Cargo")
class SourceBuildTests(unittest.TestCase):
    def test_independent_builds_use_commit_epoch_and_keep_the_offline_pruned_lock(self):
        version = "1.2.3-rc.1"
        epoch = 1234567890
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp).resolve()
            repo = work / "checkout"
            crate = repo / "crates/rusty-bacnet"
            omitted = repo / "crates/omitted"
            for package in (crate, omitted):
                (package / "src").mkdir(parents=True)
                (package / "Cargo.toml").write_text(
                    f'[package]\nname="{package.name}"\nversion="{version}"\nedition="2021"\n')
                (package / "src/main.rs").write_text("fn main() {}\n")
            (repo / "Cargo.toml").write_text('[workspace]\nresolver="2"\nmembers=["crates/*"]\n')
            (crate / "pyproject.toml").write_text(
                '[build-system]\nrequires=["maturin"]\nbuild-backend="maturin"\n'
                '[project]\nname="ace-rusty-bacnet"\ndynamic=["version"]\nlicense-files=["THIRD-PARTY-NOTICES"]\n'
                '[tool.maturin]\nbindings="bin"\nlocked=true\n')
            (crate / "THIRD-PARTY-NOTICES").write_text("fixture notice\n")
            shutil.copy(ROOT / "rust-toolchain.toml", repo / "rust-toolchain.toml")
            (repo / ".github").mkdir()
            shutil.copy(ROOT / ".github/ci-pins.env", repo / ".github/ci-pins.env")
            env = {**os.environ, "CARGO_NET_OFFLINE": "true", "CARGO_TARGET_DIR": str(work / "target"),
                   "GIT_AUTHOR_DATE": f"{epoch} +0000", "GIT_COMMITTER_DATE": f"{epoch} +0000",
                   "GIT_AUTHOR_NAME": "Release Test", "GIT_AUTHOR_EMAIL": "test@example.invalid",
                   "GIT_COMMITTER_NAME": "Release Test", "GIT_COMMITTER_EMAIL": "test@example.invalid",
                   "SOURCE_DATE_EPOCH": "999999999"}  # the producer must override the caller's clock
            def run(*args, cwd=repo, extra=None):
                return subprocess.run(args, cwd=cwd, env={**env, **(extra or {})}, check=True, capture_output=True, text=True)
            run("cargo", "generate-lockfile", "--offline")
            before = (repo / "Cargo.lock").read_bytes()
            run("git", "init", "-q")
            run("git", "add", ".")
            run("git", "commit", "-qm", "fixture source")
            # If repair escapes the command environment during extraction,
            # it will pick this shadow Cargo instead of the release compiler.
            incoming = {"RUSTUP_TOOLCHAIN": "caller-override"}
            if shutil.which("rustup"):
                shadow = work / "system-bin"
                shadow.mkdir()
                ambient_cargo = shadow / "cargo"
                ambient_cargo.write_text("#!/bin/sh\necho unexpected ambient Cargo >&2\nexit 29\n")
                ambient_cargo.chmod(0o755)
                incoming["PATH"] = str(shadow) + os.pathsep + env["PATH"]
                incoming["CARGO"] = incoming["RUSTC"] = str(ambient_cargo)
            archives = []
            for run_id in (1, 2):
                # Different output paths and source mtimes, same file content and commit.
                os.utime(crate / "src/main.rs", (epoch + run_id, epoch + run_id))
                output = work / str(run_id)
                result = run(sys.executable, ROOT / "scripts/release/build_ace_sdist.py", version, output, extra=incoming)
                archive = Path(result.stdout.strip().splitlines()[-1])
                self.assertEqual(archive, output / "ace_rusty_bacnet-1.2.3rc1.tar.gz")
                archives.append(archive)
                raw = archive.read_bytes()
                self.assertEqual(int.from_bytes(raw[4:8], "little"), epoch)
                with tarfile.open(archive) as source:
                    members = source.getmembers()
                    self.assertTrue(all(member.mtime == epoch for member in members))
                    root = "ace_rusty_bacnet-1.2.3rc1"
                    self.assertEqual(source.extractfile(f"{root}/THIRD-PARTY-NOTICES").read(), b"fixture notice\n")
                    lock = tomllib.loads(source.extractfile(f"{root}/Cargo.lock").read().decode())
                    self.assertEqual([(pkg["name"], pkg["version"]) for pkg in lock["package"]],
                                     [("rusty-bacnet", version)])
                    source.extractall(output, filter="data")
                probe = ROOT / "scripts/release/release_toolchain.py"
                run(sys.executable, probe, output / root, "cargo", "check", "--locked", "--offline",
                    "-p", "rusty-bacnet", extra=incoming)
            first, second = (archive.read_bytes() for archive in archives)
            self.assertEqual(hashlib.sha256(first).digest(), hashlib.sha256(second).digest())
            self.assertEqual(gzip.decompress(first), gzip.decompress(second))  # all tar headers and member bytes
            self.assertEqual((repo / "Cargo.lock").read_bytes(), before)
