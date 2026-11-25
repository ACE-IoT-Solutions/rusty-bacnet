"""Verify which Rust executables an archive build actually inherits."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("release_toolchain.py").resolve()


class ReleaseToolchainTests(unittest.TestCase):
    def test_extracted_command_uses_verified_pin_with_or_without_rustup(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            checkout = root / "checkout"
            archive = root / "archive"
            system = root / "system"
            pinned = root / "pinned"
            for directory in (checkout, archive, system, pinned):
                directory.mkdir()
            (checkout / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.0"\n')
            def executable(path, script):
                path.write_text("#!/bin/sh\n" + script + "\n")
                path.chmod(0o755)
            for directory, version in ((pinned, "1.99.0"), (system, "1.95.0")):
                for tool in ("cargo", "rustc"):
                    executable(directory / tool, f'echo "{tool} {version} (test)"')
            rustup = system / "rustup"
            executable(rustup, f'[ "$1 $2 $3" = "which --toolchain 1.99.0" ] || exit 7\necho "{pinned}/$4"')
            probe = ("import json,os,shutil; print(json.dumps([os.getcwd(),os.environ['RUSTUP_TOOLCHAIN'],"
                     "shutil.which('cargo'),os.environ['CARGO'],shutil.which('rustc'),os.environ['RUSTC']]))")
            def run(path):
                env = {**os.environ, "PATH": str(path), "RUSTUP_TOOLCHAIN": "1.95.0",
                       "CARGO": str(system / "cargo"), "RUSTC": str(system / "rustc")}
                return subprocess.run([sys.executable, SCRIPT, archive, sys.executable, "-c", probe],
                                      env=env, cwd=checkout, capture_output=True, text=True)
            def assert_uses_pin(result):
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout), [str(archive.resolve()), "1.99.0",
                                 str(pinned / "cargo"), str(pinned / "cargo"),
                                 str(pinned / "rustc"), str(pinned / "rustc")])
            assert_uses_pin(run(system))
            rustup.unlink()
            self.assertIn("expected cargo 1.99.0", run(system).stderr)
            assert_uses_pin(run(pinned))
            executable(pinned / "rustc", 'echo "rustc 1.95.0 (test)"')
            self.assertIn("expected rustc 1.99.0", run(pinned).stderr)
