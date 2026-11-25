"""Run the private release job's guards and its upload command with local git and a fake index."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

import ace_index
from release_api import local_assets, sha256_bytes, sums_text

ROOT = Path(__file__).resolve().parents[2]
VERSION = "1.2.3-rc.1"
TAG = f"v{VERSION}"


def git(repo, *arguments):
    return subprocess.check_output(["git", "-C", str(repo), *arguments], text=True,
                                   stderr=subprocess.PIPE).strip()


class PrivateReleaseTests(unittest.TestCase):
    def setUp(self):
        work = tempfile.TemporaryDirectory()
        self.addCleanup(work.cleanup)
        self.work = Path(work.name)
        self.repo = self.work / "source"
        self.origin = self.work / "origin.git"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Release test")
        git(self.repo, "config", "user.email", "test@example.invalid")
        git(self.repo, "init", "--bare", str(self.origin))
        git(self.repo, "remote", "add", "origin", str(self.origin))
        scripts = self.repo / "scripts/release"
        scripts.mkdir(parents=True)
        for name in ("ace_index.py", "release_api.py", "check_artifacts.py"):
            shutil.copy(ROOT / "scripts/release" / name, scripts)
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "tested source")
        self.commit = git(self.repo, "rev-parse", "HEAD")

    def push_tag(self, *options):
        git(self.repo, "tag", *options, TAG)
        git(self.repo, "push", "origin", f"refs/tags/{TAG}")

    def verify(self):
        return subprocess.run([sys.executable, "scripts/release/ace_index.py", "verify-source",
                               VERSION, self.commit], cwd=self.repo, capture_output=True, text=True)

    def test_remote_lightweight_and_annotated_tags_peel_to_the_tested_commit(self):
        for options in ((), ("-a", "-m", "tested release")):
            with self.subTest(options=options):
                self.push_tag(*options)
                self.assertEqual(self.verify().returncode, 0)
                git(self.repo, "push", "origin", "--delete", TAG)
                self.assertNotEqual(self.verify().returncode, 0)  # stale local tag alone
                git(self.repo, "tag", "-d", TAG)

    def test_malformed_ambiguous_and_unrelated_remote_records_are_refused(self):
        ref = f"refs/tags/{TAG}"
        base = f"{'a' * 40}\t{ref}\n"
        peeled = f"{self.commit}\t{ref}^{{}}\n"
        for response in (base + base, base + peeled + peeled, peeled, base + "unparsed",
                         base.replace(ref, ref + "-other"), base.replace("a" * 40, "0" * 41),
                         base.replace("a" * 40, "z" * 40), base.replace("\t", " ")):
            with self.subTest(response=response), self.assertRaises(ValueError):
                ace_index.advertised_tag(response, ref, 40)

    def test_missing_inaccessible_and_wrong_remote_commit_or_checkout_are_refused(self):
        self.assertNotEqual(self.verify().returncode, 0)
        self.push_tag()
        git(self.repo, "commit", "--allow-empty", "-qm", "other")
        self.assertIn("checked-out HEAD", self.verify().stderr)
        self.commit = git(self.repo, "rev-parse", "HEAD")
        self.assertIn("does not resolve", self.verify().stderr)
        git(self.repo, "checkout", "-q", "HEAD^")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        path = self.repo / "scripts/release/check_artifacts.py"
        path.write_text(path.read_text() + "\n")
        self.assertIn("tracked release source has changed", self.verify().stderr)
        git(self.repo, "checkout", "--", ".")
        git(self.repo, "remote", "set-url", "origin", str(self.work / "absent.git"))
        self.assertNotEqual(self.verify().returncode, 0)

    def test_upload_step_rechecks_tag_and_staged_bytes_before_invoking_uv(self):
        # Run the actual shell block through its upload. No network requests
        # are made: the fake uv records its arguments and stops at that boundary.
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        step = workflow.split("      - name: Publish and verify the private release\n", 1)[1]
        block = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n  # ", 1)[0])
        release = self.repo / "release"
        release.mkdir()
        stem = "ace_rusty_bacnet-1.2.3rc1"
        names = [f"{stem}.tar.gz"]
        for tag in ("cp310", "cp313"):
            for platform in ace_index.PLATFORMS:
                names.append(f"{stem}-{tag}-{tag}-{platform}.whl")
        for name in [*names, "THIRD-PARTY-NOTICES", "bacnet-linux-amd64.mcpb"]:
            (release / name).write_bytes(name.encode())
        sums = sha256_bytes(sums_text(local_assets(release)).encode())
        tools = self.work / "tools"
        tools.mkdir()
        marker = self.work / "uploads"
        for name, script in [("curl", "exit 0"), ("uv", 'printf "%s\\n" "$@" > "$UPLOAD_LOG"; exit 67')]:
            tool = tools / name
            tool.write_text("#!/bin/sh\n" + script + "\n")
            tool.chmod(0o755)
        env = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ["PATH"],
               "RUNNER_TEMP": str(self.work), "VERSION": VERSION, "STAGED_SUMS": sums,
               "PRIVATE_INDEX_URL": "https://example.invalid/simple/", "TAILSCALE_ACL_TAG": "test",
               "PRIVATE_PUBLISH_URL": "https://example.invalid/", "UPLOAD_LOG": str(marker),
               "RELEASE_COMMIT": self.commit}
        def run():
            return subprocess.run(["bash", "-eo", "pipefail", "-c", block], cwd=self.repo,
                                  env=env, capture_output=True, text=True)

        self.push_tag()
        self.assertEqual(self.verify().returncode, 0)
        for name in (names[0], "bacnet-linux-amd64.mcpb"):
            with self.subTest(changed=name):
                path = release / name
                path.write_bytes(b"replacement")
                result = run()
                self.assertIn("staged SHA256SUMS", result.stderr)
                self.assertFalse(marker.exists())
                path.write_bytes(name.encode())
        git(self.repo, "push", "origin", "--delete", TAG)
        self.assertNotEqual(run().returncode, 0)
        self.assertFalse(marker.exists())
        git(self.repo, "push", "origin", f"refs/tags/{TAG}")
        result = run()
        self.assertEqual(result.returncode, 67, result.stderr)
        args = marker.read_text().splitlines()
        self.assertIn("--check-url", args)
        self.assertCountEqual([name for name in args if name.startswith("release/")],
                              ["release/" + name for name in names])
