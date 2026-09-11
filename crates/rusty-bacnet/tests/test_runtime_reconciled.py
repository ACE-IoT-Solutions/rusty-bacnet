"""Focused contract tests for the reconciled coarse BACnet runtime binding."""

import asyncio
import inspect
import unittest

import rusty_bacnet as bacnet


class RuntimeBindingContractTests(unittest.TestCase):
    def test_runtime_surface_is_coarse_and_results_are_immutable(self) -> None:
        for method in (
            "start",
            "reconcile",
            "restore_devices",
            "discover",
            "scan",
            "load_property_catalog",
            "scan_device",
            "topology",
            "health",
            "read_batch",
            "submit_read_batch",
            "write_batch",
            "submit_write_batch",
            "apply_observation_plan",
            "cancel",
            "next_events",
            "stop",
        ):
            self.assertTrue(callable(getattr(bacnet.BACnetRuntime, method)))

        self.assertFalse(hasattr(bacnet.BACnetRuntime, "read_property"))
        self.assertFalse(hasattr(bacnet.BACnetRuntime, "read_property_multiple"))

        attachment = bacnet.RuntimeAttachment(
            7, "immutable", "127.0.0.1", 0, "127.255.255.255"
        )
        with self.assertRaises(AttributeError):
            attachment.label = "mutated"

        persisted = bacnet.RuntimePersistedDevice(
            7,
            100,
            [127, 0, 0, 1, 0xBA, 0xC0],
            routed_dnet=2200,
            routed_dadr=[0, 5],
        )
        self.assertEqual(persisted.attachment_id, 7)
        self.assertEqual(persisted.routed_dnet, 2200)
        self.assertEqual(persisted.routed_dadr, b"\x00\x05")
        with self.assertRaises(AttributeError):
            persisted.attachment_id = 8

    def test_runtime_sc_credentials_are_enforced_by_native_start(self) -> None:
        parameters = inspect.signature(bacnet.RuntimeScAttachment).parameters
        self.assertIn("ca_cert", parameters)
        self.assertIn("client_cert", parameters)
        self.assertIn("client_key", parameters)

        with self.assertRaises(TypeError):
            bacnet.RuntimeScAttachment(
                10,
                "missing-credentials",
                "wss://localhost:47808",
                b"\x01\x02\x03\x04\x05\x06",
            )

        attachment = bacnet.RuntimeScAttachment(
            10,
            "empty-credentials",
            "wss://localhost:47808",
            b"\x01\x02\x03\x04\x05\x06",
            "",
            "",
            "",
        )

        async def check() -> None:
            with self.assertRaises(RuntimeError) as raised:
                await bacnet.BACnetRuntime.start([attachment])
            self.assertEqual(raised.exception.code, "InvalidConfig")
            self.assertEqual(raised.exception.attachment_id, 10)

        asyncio.run(check())

    def test_reconcile_and_restore_preserve_attachment_scoped_ownership(self) -> None:
        async def check() -> None:
            primary = bacnet.RuntimeAttachment(1, "primary", "127.0.0.1", 0)
            temporary = bacnet.RuntimeAttachment(2, "temporary", "127.0.0.1", 0)
            runtime = await bacnet.BACnetRuntime.start([primary])
            try:
                replay = await runtime.reconcile(1, [primary])
                self.assertTrue(replay.idempotent)
                self.assertEqual(replay.generation, 1)

                added = await runtime.reconcile(2, [primary, temporary])
                self.assertEqual(added.added, [2])
                self.assertEqual(added.updated, [])
                self.assertEqual(added.removed, [])

                restored = await runtime.restore_devices(
                    [
                        bacnet.RuntimePersistedDevice(
                            1, 100, [127, 0, 0, 1, 0xBA, 0xC0]
                        ),
                        bacnet.RuntimePersistedDevice(
                            2,
                            101,
                            [127, 0, 0, 2, 0xBA, 0xC1],
                            routed_dnet=2200,
                            routed_dadr=[0, 5],
                        ),
                    ]
                )
                self.assertEqual(
                    (restored.added, restored.updated, restored.unchanged),
                    (2, 0, 0),
                )
                self.assertEqual((await runtime.health()).device_count, 2)

                removed = await runtime.reconcile(3, [primary])
                self.assertEqual(removed.removed, [2])
                self.assertEqual((await runtime.health()).attachment_ids, [1])
            finally:
                await runtime.stop()

        asyncio.run(check())


if __name__ == "__main__":
    unittest.main()
