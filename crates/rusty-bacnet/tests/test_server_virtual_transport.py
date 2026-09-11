"""Focused coverage for the Python server's named virtual transport."""

from __future__ import annotations

import inspect
import unittest
import uuid

from rusty_bacnet import BACnetServer, BacnetError


class VirtualServerConfigurationTests(unittest.TestCase):
    def test_constructor_exposes_keyword_only_virtual_configuration(self) -> None:
        parameters = inspect.signature(BACnetServer).parameters
        for name in ("virtual_network", "virtual_mac"):
            self.assertIn(name, parameters)
            self.assertEqual(parameters[name].kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(parameters[name].default)

    def test_virtual_configuration_is_transport_scoped_and_complete(self) -> None:
        with self.assertRaisesRegex(ValueError, "virtual_network"):
            BACnetServer(510001, transport="virtual", virtual_mac=7)
        with self.assertRaisesRegex(ValueError, "virtual_mac"):
            BACnetServer(510002, transport="virtual", virtual_network="test")
        with self.assertRaisesRegex(ValueError, "non-blank"):
            BACnetServer(
                510003,
                transport="virtual",
                virtual_network="  ",
                virtual_mac=7,
            )
        with self.assertRaisesRegex(ValueError, "require transport='virtual'"):
            BACnetServer(510004, virtual_network="test", virtual_mac=7)


class VirtualServerLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def test_start_stop_restart_releases_and_rejoins_membership(self) -> None:
        network = f"server-lifecycle-{uuid.uuid4()}"
        server = BACnetServer(
            510005,
            transport="virtual",
            virtual_network=network,
            virtual_mac=0x2A,
        )

        try:
            await server.start()
            self.assertEqual(await server.local_address(), "2a")
            with self.assertRaisesRegex(BacnetError, "already joined"):
                duplicate = BACnetServer(
                    510006,
                    transport="virtual",
                    virtual_network=network,
                    virtual_mac=0x2A,
                )
                await duplicate.start()

            await server.stop()
            await server.start()
            self.assertEqual(await server.local_address(), "2a")
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
