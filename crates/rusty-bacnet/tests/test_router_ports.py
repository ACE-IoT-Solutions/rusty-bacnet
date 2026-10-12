"""Installed-wheel contract tests for typed mixed-transport router ports."""

from __future__ import annotations

import inspect
import unittest
import uuid

from rusty_bacnet import (
    BACnetRouter,
    RouterBipPort,
    RouterPortHealth,
    RouterRouteEntry,
    RouterScPort,
)


class RouterPortConfigTests(unittest.IsolatedAsyncioTestCase):
    def test_sc_validation_is_fail_closed(self) -> None:
        valid = dict(
            network_number=200,
            primary_hub="wss://localhost:47808",
            local_vmac=b"\x01\x02\x03\x04\x05\x06",
            device_uuid=uuid.uuid4().bytes,
            ca_cert="ca.pem",
            client_cert="client.pem",
            client_key="client.key",
        )
        configured = RouterScPort(**valid)
        parameters = inspect.signature(RouterScPort).parameters
        self.assertIn("reconnect_forever", parameters)
        self.assertNotIn("retry_forever", parameters)
        self.assertFalse(configured.reconnect_forever)
        self.assertEqual(configured.max_npdu_length, 1497)
        self.assertEqual(configured.max_bvlc_length, 5705)
        self.assertIsInstance(configured.local_vmac, bytes)
        self.assertIsInstance(configured.device_uuid, bytes)
        with self.assertRaises(AttributeError):
            configured.network_number = 201
        for change in (
            {"network_number": 0},
            {"primary_hub": "ws://localhost:47808"},
            {"local_vmac": b"\0" * 6},
            {"local_vmac": b"\xff" * 6},
            {"device_uuid": b"\0" * 16},
            {"ca_cert": ""},
            {"heartbeat_interval_ms": 2999},
            {"heartbeat_timeout_ms": 30000},
            {"reconnect_initial_delay_ms": 0},
            {"reconnect_initial_delay_ms": 20, "reconnect_max_delay_ms": 10},
            {"reconnect_max_retries": 0},
            {"max_npdu_length": 1496},
            {"max_bvlc_length": 5704},
            {"max_npdu_length": 61328, "max_bvlc_length": 65535},
        ):
            with self.subTest(change=change), self.assertRaises(ValueError):
                RouterScPort(**(valid | change))

    def test_from_ports_validation(self) -> None:
        bip = RouterBipPort(100, interface="127.0.0.1", port=0)
        sc = RouterScPort(
            200,
            "wss://localhost:47808",
            b"\x01\x02\x03\x04\x05\x06",
            uuid.uuid4().bytes,
            "ca.pem",
            "client.pem",
            "client.key",
        )
        router = BACnetRouter.from_ports([bip, sc])
        self.assertIsInstance(router, BACnetRouter)
        self.assertIn(
            "announce_interval_s", inspect.signature(BACnetRouter.from_ports).parameters
        )
        self.assertIsInstance(
            BACnetRouter.from_ports([bip, sc], announce_interval_s=60), BACnetRouter
        )
        with self.assertRaises(ValueError):
            BACnetRouter.from_ports([bip])
        for invalid_interval in (0, -1, float("inf"), float("nan")):
            with self.subTest(announce_interval_s=invalid_interval), self.assertRaises(ValueError):
                BACnetRouter.from_ports([bip, sc], announce_interval_s=invalid_interval)
        with self.assertRaises(ValueError):
            BACnetRouter.from_ports(
                [bip, RouterBipPort(300, interface="127.0.0.1", port=0)]
            )

    def test_snapshot_types_are_exported(self) -> None:
        self.assertTrue(inspect.isclass(RouterPortHealth))
        self.assertTrue(inspect.isclass(RouterRouteEntry))


if __name__ == "__main__":
    unittest.main()
