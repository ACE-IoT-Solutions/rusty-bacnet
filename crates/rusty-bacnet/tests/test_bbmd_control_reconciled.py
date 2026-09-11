"""Installed-extension tests for local BBMD control and bounded policy."""

from __future__ import annotations

import asyncio
import socket
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetServer,
    BacnetBbmdControlError,
    BbmdControl,
    BbmdSnapshot,
    BvllPolicyContext,
    BvllPolicyVerdict,
)


def _unused_udp_port() -> int:
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    probe.bind(("127.0.0.1", 0))
    port = probe.getsockname()[1]
    probe.close()
    return port


class BbmdConfigurationTests(unittest.TestCase):
    def test_fail_closed_configuration_validation(self) -> None:
        with self.assertRaises(ValueError):
            BACnetServer(7001, transport="ipv6", bbmd=True)
        with self.assertRaises(ValueError):
            BACnetServer(7001, bbmd_bdt=[])
        with self.assertRaises(ValueError):
            BACnetServer(7001, bbmd=True, bbmd_wire_management_enabled=True)
        with self.assertRaises(ValueError):
            BACnetServer(7001, bbmd=True, bbmd_max_fdt_entries=129)

        async def async_policy(_context: BvllPolicyContext) -> str:
            return "continue"

        with self.assertRaises(ValueError):
            BACnetServer(7001, bbmd=True, bvll_policy=async_policy)

    def test_verdict_objects_are_narrowing_only(self) -> None:
        self.assertEqual(BvllPolicyVerdict.continue_native().kind, "continue")
        self.assertEqual(BvllPolicyVerdict.drop().kind, "drop")
        rejected = BvllPolicyVerdict.reject(0x0010)
        self.assertEqual(rejected.kind, "reject")
        self.assertEqual(rejected.result_code, 0x0010)


class BbmdControlTests(unittest.IsolatedAsyncioTestCase):
    async def test_lifecycle_snapshot_atomic_persistence_and_policy(self) -> None:
        contexts: list[BvllPolicyContext] = []

        def policy(context: BvllPolicyContext) -> str:
            contexts.append(context)
            return "drop"

        with tempfile.TemporaryDirectory() as directory:
            persist_path = Path(directory) / "bdt.bin"
            port = _unused_udp_port()
            server = BACnetServer(
                7002,
                interface="127.0.0.1",
                port=port,
                broadcast_address="127.0.0.1",
                bbmd=True,
                bbmd_bdt_persist_path=str(persist_path),
                bbmd_accept_foreign_devices=True,
                bbmd_max_fdt_entries=7,
                bbmd_management_acl=["127.0.0.1"],
                bvll_policy=policy,
            )
            control = server.bbmd_control
            self.assertIsInstance(control, BbmdControl)
            self.assertEqual(control.lifecycle, "not_started")
            with self.assertRaises(BacnetBbmdControlError) as error:
                await control.snapshot()
            self.assertEqual(error.exception.code, "not_started")

            await server.start()
            try:
                snapshot = await control.snapshot()
                self.assertIsInstance(snapshot, BbmdSnapshot)
                self.assertFalse(snapshot.wire_bdt_writes_enabled)
                self.assertTrue(snapshot.accept_foreign_devices)
                self.assertEqual(snapshot.max_fdt_entries, 7)
                self.assertEqual(snapshot.management_acl, ["127.0.0.1"])

                revision = await control.replace_bdt(
                    [("192.0.2.17", 47808, "255.255.255.255")]
                )
                self.assertEqual(revision, 1)
                snapshot = await control.snapshot()
                self.assertEqual(snapshot.revision, revision)
                self.assertEqual(snapshot.bdt[0].ip, "192.0.2.17")
                self.assertEqual(len(persist_path.read_bytes()), 10)

                sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    # Structurally valid Original-Unicast-NPDU reaches the policy
                    # after the native cheap source/function gate.
                    sender.sendto(b"\x81\x0a\x00\x08\x01\x00\x10\x00", ("127.0.0.1", port))
                    for _ in range(50):
                        if control.policy_counters().evaluated:
                            break
                        await asyncio.sleep(0.02)
                    counters = control.policy_counters()
                    self.assertEqual(counters.evaluated, 1)
                    self.assertEqual(counters.dropped, 1)
                    self.assertEqual(contexts[0].function, 0x0A)
                    self.assertLessEqual(len(contexts[0].payload_prefix), 256)
                finally:
                    sender.close()
            finally:
                await server.stop()

            self.assertEqual(control.lifecycle, "stopped")
            with self.assertRaises(BacnetBbmdControlError) as error:
                await control.snapshot()
            self.assertEqual(error.exception.code, "stopped")


if __name__ == "__main__":
    unittest.main()
