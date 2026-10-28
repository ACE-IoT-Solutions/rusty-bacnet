"""Installed-wheel contract tests for typed mixed-transport router ports."""

from __future__ import annotations

import asyncio
import inspect
import socket
import unittest
import uuid

from rusty_bacnet import (
    AceMixedRouter,
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
        self.assertFalse(configured.reconnect_forever)
        self.assertEqual(configured.max_npdu_length, 1497)
        self.assertEqual(configured.max_bvlc_length, 5705)
        with self.assertRaises(AttributeError):
            configured.network_number = 201
        for change in (
            {"network_number": 0},
            {"primary_hub": "ws://localhost:47808"},
            {"primary_hub": "wss://:47808"},
            {"primary_hub": "wss://[::1"},
            {"local_vmac": b"\0" * 6},
            {"local_vmac": b"\xff" * 6},
            {"device_uuid": b"\0" * 16},
            {"ca_cert": ""},
            {"heartbeat_interval_ms": 2999},
            {"heartbeat_timeout_ms": 30000},
            {"reconnect_initial_delay_ms": 0},
            {"reconnect_initial_delay_ms": 20, "reconnect_max_delay_ms": 10},
            {"reconnect_max_delay_ms": 86_400_001},
            {"reconnect_max_retries": 0},
            {"failover_hub": "wss://failover.local:47808"},
            {"max_npdu_length": 1496},
            {"max_bvlc_length": 5704},
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
        self.assertIsInstance(AceMixedRouter.from_ports([bip, sc]), AceMixedRouter)
        with self.assertRaises(ValueError):
            AceMixedRouter.from_ports([bip])
        for interval in (0, 0.5, -1, float("inf"), float("nan")):
            with self.subTest(interval=interval), self.assertRaises(ValueError):
                AceMixedRouter.from_ports([bip, sc], announce_interval_s=interval)

    async def test_bip_router_lifecycle_and_snapshots(self) -> None:
        router = AceMixedRouter.from_ports(
            [
                RouterBipPort(100, interface="127.0.0.1", port=0),
                RouterBipPort(200, interface="127.0.0.1", port=0),
            ]
        )
        self.assertIsNone(await router.start())
        self.assertTrue((await router.local_address()).startswith("127.0.0.1:"))
        health = await router.port_health()
        self.assertEqual([entry.state for entry in health], ["Up", "Up"])
        routes = await router.routing_table()
        self.assertEqual([entry.network_number for entry in routes], [100, 200])
        self.assertTrue(all(entry.directly_connected for entry in routes))
        with self.assertRaises(RuntimeError):
            await router.start()
        self.assertIsNone(await router.stop())
        self.assertEqual(
            [entry.state for entry in await router.port_health()], ["Up", "Up"]
        )
        self.assertEqual(len(await router.routing_table()), 2)
        self.assertIsNone(await router.stop())
        self.assertIsNone(await router.start())
        self.assertIsNone(await router.stop())

    async def test_concurrent_lifecycle_is_single_owner(self) -> None:
        router = AceMixedRouter.from_ports(
            [
                RouterBipPort(100, interface="127.0.0.1", port=0),
                RouterBipPort(200, interface="127.0.0.1", port=0),
            ]
        )
        results = await asyncio.gather(
            router.start(), router.start(), return_exceptions=True
        )
        self.assertEqual(sum(result is None for result in results), 1)
        self.assertEqual(sum(isinstance(result, RuntimeError) for result in results), 1)
        self.assertIsNone(await router.stop())

        starting = asyncio.ensure_future(router.start())
        await asyncio.sleep(0)
        stopping = asyncio.ensure_future(router.stop())
        started, stopped = await asyncio.gather(
            starting, stopping, return_exceptions=True
        )
        # A close admitted before startup publishes wins. Either an already
        # published success or explicit cancellation is valid; stop is the
        # cleanup barrier in both cases.
        if isinstance(started, RuntimeError):
            self.assertIn("startup was cancelled", str(started))
        else:
            self.assertIsNone(started)
        self.assertIsNone(stopped)
        with self.assertRaises(RuntimeError):
            await router.local_address()
        # The retained stop is a resource-release barrier even when it wins
        # the race with startup publication.
        for health in await router.port_health():
            bound_port = int(health.identity.rsplit(":", 1)[1])
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
                probe.bind(("0.0.0.0", bound_port))
        self.assertIsNone(await router.start())
        self.assertIsNone(await router.stop())

    def test_snapshot_types_are_exported(self) -> None:
        self.assertTrue(inspect.isclass(RouterPortHealth))
        self.assertTrue(inspect.isclass(RouterRouteEntry))


if __name__ == "__main__":
    unittest.main()
