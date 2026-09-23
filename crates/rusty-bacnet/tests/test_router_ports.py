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
    RouterVirtualPort,
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
        ):
            with self.subTest(change=change), self.assertRaises(ValueError):
                RouterScPort(**(valid | change))

    def test_from_ports_validation(self) -> None:
        bip = RouterBipPort(100, interface="127.0.0.1", port=0)
        virtual = RouterVirtualPort(200, "typed-router", 1)
        router = BACnetRouter.from_ports([bip, virtual])
        self.assertIsInstance(router, BACnetRouter)
        self.assertNotIn(
            "announce_interval_s", inspect.signature(BACnetRouter.from_ports).parameters
        )
        with self.assertRaises(ValueError):
            BACnetRouter.from_ports([bip])
        with self.assertRaises(TypeError):
            BACnetRouter.from_ports([bip, virtual], announce_interval_s=60)
        with self.assertRaises(ValueError):
            BACnetRouter.from_ports(
                [bip, RouterBipPort(300, interface="127.0.0.1", port=0)]
            )

    async def test_virtual_health_routes_and_stopped_snapshots(self) -> None:
        suffix = uuid.uuid4().hex
        router = BACnetRouter.from_ports(
            [
                RouterBipPort(
                    101,
                    interface="127.0.0.1",
                    port=0,
                    broadcast_address="127.0.0.1",
                ),
                RouterVirtualPort(201, f"typed-{suffix}", 1),
            ]
        )
        await router.start()
        health = await router.port_health()
        routes = await router.routing_table()
        self.assertEqual([item.network_number for item in health], [101, 201])
        self.assertTrue(all(isinstance(item, RouterPortHealth) for item in health))
        self.assertEqual([item.network_number for item in routes], [101, 201])
        self.assertTrue(all(isinstance(item, RouterRouteEntry) for item in routes))
        self.assertTrue(all(item.directly_connected for item in routes))
        self.assertTrue(all(isinstance(item.next_hop_mac, bytes) for item in routes))
        await router.stop()
        self.assertEqual(len(await router.port_health()), 2)
        self.assertEqual(len(await router.routing_table()), 2)


if __name__ == "__main__":
    unittest.main()
