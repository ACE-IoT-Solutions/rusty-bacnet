"""Installed-wheel coverage for Notification Forwarder registration."""

from __future__ import annotations

import asyncio
import inspect
import unittest

import rusty_bacnet as bacnet


class NotificationForwarderRegistrationTests(unittest.TestCase):
    def test_registration_signature(self) -> None:
        parameters = inspect.signature(
            bacnet.BACnetServer.add_notification_forwarder
        ).parameters
        self.assertEqual(list(parameters), ["self", "instance", "name"])

    def test_registered_property_model_is_readable(self) -> None:
        async def run() -> None:
            server = bacnet.BACnetServer(
                4_180_051,
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            )
            server.add_notification_forwarder(7, "NF-7")
            await server.start()
            try:
                oid = bacnet.ObjectIdentifier(
                    bacnet.ObjectType.NOTIFICATION_FORWARDER, 7
                )
                expected = {
                    bacnet.PropertyIdentifier.OBJECT_NAME: "NF-7",
                    bacnet.PropertyIdentifier.PROCESS_IDENTIFIER_FILTER: [],
                    bacnet.PropertyIdentifier.SUBSCRIBED_RECIPIENTS: 0,
                    bacnet.PropertyIdentifier.LOCAL_FORWARDING_ONLY: False,
                    bacnet.PropertyIdentifier.EVENT_DETECTION_ENABLE: True,
                }
                for property_id, value in expected.items():
                    actual = await server.read_property(oid, property_id)
                    self.assertEqual(actual.value, value)
            finally:
                await server.stop()

        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
