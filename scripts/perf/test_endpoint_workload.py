"""Run the measurement CLI and its separate peer on an installed release wheel."""

import json
import os
import socket
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile


SCRIPT = Path(__file__).with_name("endpoint_workload.py")
WHEEL = os.environ.get("ENDPOINT_WORKLOAD_TEST_WHEEL")


class PeerTests(unittest.TestCase):
    def test_peer_rejects_wrong_initial_segment_ack_and_keeps_counts(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as endpoint:
            endpoint.bind(("127.0.0.1", 0))
            endpoint.settimeout(10)
            with subprocess.Popen([sys.executable, SCRIPT.with_name("endpoint_workload_peer.py"),
                                   str(endpoint.getsockname()[1]), "31", "1"], stdout=subprocess.PIPE) as peer:
                try:
                    router = ("127.0.0.1", json.loads(peer.stdout.readline())["port"])
                    # Same routed RP that the benchmark requests. ACK1 in response to COV segment 0.
                    endpoint.sendto(bytes.fromhex("810a0016 01240005010aff 0205010c 0c00000001191c"), router)
                    segment, _ = endpoint.recvfrom(2048)
                    self.assertEqual(segment[10:16], bytes.fromhex("0c0501000201"))
                    endpoint.sendto(bytes.fromhex("810a000f 01200005010aff 41010102"), router)
                    self.assertEqual(peer.wait(timeout=10), 1)
                    report = json.loads(peer.stdout.readline())
                    self.assertIn("unexpected cov SegmentACK at 0", report["error"])
                    self.assertEqual(report["counts"]["rx_packets"], 2)
                    self.assertEqual(report["counts"]["tx_segment_cov"], 1)
                    self.assertEqual(report["acks"]["cov"], [])
                finally:
                    peer.kill()


@unittest.skipUnless(sys.platform.startswith("linux") or sys.platform == "darwin", "requires Linux or macOS")
@unittest.skipUnless(WHEEL, "set ENDPOINT_WORKLOAD_TEST_WHEEL to the installed Rusty wheel")
class WorkloadTests(unittest.TestCase):
    def sample(self, wheel, output):
        result = subprocess.run([sys.executable, SCRIPT, "--wheel", wheel, "--source-sha", "a" * 40,
                                 "--seed", "31", "--warmup", "1", "--rounds", "2", "--output", output],
                                timeout=90, capture_output=True, text=True)
        return result, json.loads(output.read_text())

    def test_three_complete_exchanges_and_exact_artifact_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            result, report = self.sample(WHEEL, out / "ok.json")
            self.assertEqual(result.returncode, 0, (result.stderr, report.get("error")))
            self.assertTrue(report["success"])
            self.assertNotEqual(report["peer_pid"], report["environment"]["pid"])
            self.assertEqual(report["final"], {"active_leases": 0, "ready": True, "cov_missed": 0})
            self.assertEqual([r["phase"] for r in report["rounds"]], ["warmup", "measured", "measured"])
            digests = set()
            for index, row in enumerate(report["rounds"]):
                self.assertTrue(row["complete"])
                self.assertEqual(row["round"], index)
                ledger = row["peer"]
                self.assertEqual(sorted(ledger["read_order"]), ["rp", "rpm"])
                self.assertEqual(ledger["acks"], {k: [0, 2, 3] for k in ("rp", "rpm", "cov")})
                self.assertEqual(set(row["value_sha256"]), {"rp", "rpm", "cov"})
                digests.update(row["value_sha256"].values())
            self.assertEqual(len(digests), 9)
            counts = report["summary"]["peer_counts"]
            self.assertEqual((counts["rx_request_rp"], counts["rx_request_rpm"], counts["rx_simple_ack_cov"]), (2, 2, 2))
            self.assertEqual((counts["rx_packets"], counts["tx_packets"], counts["rx_error_reject_abort"]), (24, 24, 0))
            self.assertEqual(report["summary"]["measured_completed"], 2)
            self.assertEqual(report["summary"]["measured_failed"], 0)
            for latency in report["summary"]["latency_ns"].values():
                self.assertEqual(latency["n"], 2)
            # Matching filename/ABI with a different native binary must fail before sampling.
            wrong = out / Path(WHEEL).name
            with zipfile.ZipFile(wrong, "w") as archive:
                archive.writestr(report["artifacts"]["wheel_member"], b"different extension")
            result, failed = self.sample(wrong, out / "failed.json")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(failed["success"])
            self.assertEqual(failed["phase"], "provenance")
            self.assertNotEqual(failed["artifacts"]["wheel_member_sha256"], failed["artifacts"]["native_sha256"])
            self.assertIn("differs from wheel", failed["error"]["message"])
            self.assertEqual(failed["summary"]["measured_unstarted"], 2)
            self.assertFalse(failed["rounds"])
