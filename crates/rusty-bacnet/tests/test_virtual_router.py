"""Installed-wheel lifecycle coverage for the composable router."""

from __future__ import annotations

import asyncio
import inspect
import unittest
import uuid

from rusty_bacnet import BACnetRouter, RouterPortCounters


class VirtualRouterTests(unittest.IsolatedAsyncioTestCase):
    def make_router(self) -> BACnetRouter:
        suffix = uuid.uuid4().hex
        return BACnetRouter(
            101,
            [(1301, f"router-a-{suffix}", 1), (1302, f"router-b-{suffix}", 1)],
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )

    def test_validation_and_public_signature(self) -> None:
        with self.assertRaises(ValueError):
            BACnetRouter(0, [(1201, "valid", 1)])
        with self.assertRaises(ValueError):
            BACnetRouter(1, [(1, "valid", 1)])
        with self.assertRaises(ValueError):
            BACnetRouter(1, [(1201, "same", 1), (1202, "same", 2)])
        self.assertEqual(
            tuple(inspect.signature(BACnetRouter).parameters),
            (
                "bip_network",
                "virtual_ports",
                "interface",
                "port",
                "broadcast_address",
                "reuse_port",
            ),
        )

    async def test_start_stop_counters_and_restart(self) -> None:
        router = self.make_router()
        await router.start()
        self.assertRegex(await router.local_address(), r"^127\.0\.0\.1:\d+$")
        counters = await router.port_counters()
        self.assertEqual([item.config_index for item in counters], [0, 1, 2])
        self.assertEqual([item.network_number for item in counters], [101, 1301, 1302])
        self.assertTrue(all(isinstance(item, RouterPortCounters) for item in counters))
        with self.assertRaises(AttributeError):
            counters[0].forwarded_unicast = 1
        await router.stop()
        self.assertEqual(len(await router.port_counters()), 3)
        await router.start()
        await router.stop()

    async def test_cancelled_waiters_do_not_cancel_native_lifecycle(self) -> None:
        router = self.make_router()
        start = router.start()
        start.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await start
        await router.stop()

        await router.start()
        stop = router.stop()
        stop.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await stop
        await router.start()
        await router.stop()


if __name__ == "__main__":
    unittest.main()
