"""Exercise source-workspace lock repair with Cargo and a real tar archive."""

import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

import repair_sdist


@unittest.skipUnless(shutil.which("cargo"), "Cargo is required for the source lock regression")
class RepairTests(unittest.TestCase):
    def test_pruned_workspace_builds_locked_and_keeps_notices(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / "ace_rusty_bacnet-1.2.3"
            source.mkdir()
            manifest = source / "Cargo.toml"
            workspace = '[workspace]\nmembers = [{}]\nexclude = ["shared", "extra"]\nresolver = "2"\n'
            manifest.write_text(workspace.format('"binding", "omitted"'))
            for name in ("binding", "omitted", "shared", "extra"):
                package = source / name
                (package / "src").mkdir(parents=True)
                text = f'[package]\nname = "{name}"\nversion = "1.2.3"\nedition = "2021"\n'
                if name == "binding":
                    text += '[dependencies]\nshared = { path = "../shared" }\n'
                elif name == "omitted":
                    text += '[dependencies]\nshared = { path = "../shared", features = ["extra"] }\n'
                elif name == "shared":
                    text += '[workspace]\n[dependencies]\nextra = { path = "../extra", optional = true }\n'
                else:
                    text += '[workspace]\n'
                (package / "Cargo.toml").write_text(text)
                (package / "src/lib.rs").write_text("pub fn example() {}\n")
            command = ["cargo", "metadata", "--offline", "--format-version", "1"]
            subprocess.run(command, cwd=source, check=True, capture_output=True)
            # The omitted member enabled an optional dependency in shared.
            # Pruning it changes shared's locked edges, as the full CLI's
            # features do in the Python-only maturin source workspace.
            manifest.write_text(workspace.format('"binding"'))
            shutil.rmtree(source / "omitted")
            self.assertNotEqual(subprocess.run([*command, "--locked"], cwd=source,
                                              capture_output=True).returncode, 0)
            notices = b"retained third-party notices\n"
            (source / "THIRD-PARTY-NOTICES").write_bytes(notices)
            archive = directory / f"{source.name}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(source, arcname=source.name)
            with tarfile.open(archive) as original:
                payloads = {item.name: original.extractfile(item).read()
                            for item in original.getmembers() if item.isfile()
                            and item.name != f"{source.name}/Cargo.lock"}
            before = archive.read_bytes()
            with mock.patch.object(repair_sdist.subprocess, "run", side_effect=OSError("cargo failed")):
                with self.assertRaisesRegex(OSError, "cargo failed"):
                    repair_sdist.repair(archive)
            self.assertEqual(archive.read_bytes(), before)
            with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1234567890"}):
                repair_sdist.repair(archive)
                once = archive.read_bytes()
                repair_sdist.repair(archive)
                self.assertEqual(archive.read_bytes(), once)
            unpacked = directory / "unpacked"
            with tarfile.open(archive) as result:
                self.assertEqual(result.extractfile(f"{source.name}/THIRD-PARTY-NOTICES").read(), notices)
                self.assertEqual({item.name: result.extractfile(item).read()
                                  for item in result.getmembers() if item.isfile()
                                  and item.name != f"{source.name}/Cargo.lock"}, payloads)
                result.extractall(unpacked, filter="data")
            subprocess.run([*command, "--locked"], cwd=unpacked / source.name,
                           check=True, capture_output=True)

    def test_unsafe_archive_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                member = tarfile.TarInfo("../outside")
                member.size = 1
                output.addfile(member, io.BytesIO(b"x"))
            before = archive.read_bytes()
            with self.assertRaisesRegex(ValueError, "unsafe source archive path"):
                repair_sdist.repair(archive)
            self.assertEqual(archive.read_bytes(), before)

    def test_duplicate_members_and_escaping_links_are_refused(self):
        for duplicate in (False, True):
            with self.subTest(duplicate=duplicate), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "bad.tar.gz"
                with tarfile.open(archive, "w:gz") as output:
                    member = tarfile.TarInfo("source/link")
                    if duplicate:
                        output.addfile(member)
                    else:
                        member.type = tarfile.SYMTYPE
                        member.linkname = "../../outside"
                    output.addfile(member)
                before = archive.read_bytes()
                with self.assertRaises((ValueError, tarfile.FilterError)):
                    repair_sdist.repair(archive)
                self.assertEqual(archive.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
