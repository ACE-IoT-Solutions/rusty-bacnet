"""Installed-wheel contract checks for managed COV bindings."""

from __future__ import annotations

import inspect
import unittest

import rusty_bacnet as rb


class ManagedCOVContractTests(unittest.IsolatedAsyncioTestCase):
    def test_managed_cov_types_and_client_method_are_exported(self) -> None:
        self.assertTrue(hasattr(rb, "ManagedCOVEvent"))
        self.assertTrue(hasattr(rb, "ManagedCOVEventIterator"))
        self.assertTrue(hasattr(rb, "ManagedCOVSubscription"))
        self.assertTrue(hasattr(rb.BACnetClient, "manage_cov_subscription"))

        for member in ("closed", "finished", "last_event", "events", "close", "cancel"):
            with self.subTest(member=member):
                self.assertTrue(hasattr(rb.ManagedCOVSubscription, member))

    async def test_direct_and_routed_targets_reach_async_client_guard(self) -> None:
        client = rb.BACnetClient()
        object_id = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
        targets = (
            rb.DirectTarget("192.0.2.10:47808"),
            rb.RoutedTarget("192.0.2.1:47808", 2001, b"\x07"),
        )

        for target in targets:
            with self.subTest(target=target):
                operation = client.manage_cov_subscription(
                    target,
                    17,
                    object_id,
                    False,
                    60,
                    renewal_margin_ms=10_000,
                    event_channel_capacity=4,
                )
                self.assertTrue(inspect.isawaitable(operation))
                with self.assertRaisesRegex(RuntimeError, "client not started"):
                    await operation


if __name__ == "__main__":
    unittest.main()
