"""Installed-wheel BACnet/IP-to-BACnet/SC router acceptance."""

from __future__ import annotations

import asyncio
import socket

from rusty_bacnet import (
    BACnetClient,
    BacnetError,
    BACnetRouter,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    RoutedTarget,
    RouterBipPort,
    RouterScPort,
    RouterVirtualPort,
)

from test_sc_hub_mtls import MtlsFixture, SERVER_UUID


ROUTER_UUID = bytes.fromhex("73121b03840e49b795f6dfc6cdb7f101")
SERVER_VMAC = b"\x02\x00\x00\x00\x00\x02"
ROUTER_VMAC = b"\x02\x00\x00\x00\x00\x05"
BIP_NETWORK = 101
SC_NETWORK = 201


async def wait_for_sc_state(router: BACnetRouter, state: str, timeout: float = 8.0):
    deadline = asyncio.get_running_loop().time() + timeout
    observed: list[str] = []
    while asyncio.get_running_loop().time() < deadline:
        health = await router.port_health()
        current = health[1].state
        observed.append(current)
        if current == state:
            return health[1]
        await asyncio.sleep(0.05)
    raise AssertionError(f"SC port did not reach {state}; observed={observed}")


class ScIpRouterTests(MtlsFixture):
    async def test_bip_start_error_keeps_bacnet_error_taxonomy(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
            # BipTransport binds INADDR_ANY so reserve the wildcard endpoint;
            # a loopback-only reservation may coexist with that bind on macOS.
            reservation.bind(("0.0.0.0", 0))
            unavailable_port = reservation.getsockname()[1]
            router = BACnetRouter.from_ports(
                [
                    RouterBipPort(
                        303, interface="127.0.0.1", port=unavailable_port
                    ),
                    RouterVirtualPort(304, "sc-router-bip-start-error", 1),
                ]
            )
            with self.assertRaises(BacnetError) as raised:
                await asyncio.wait_for(router.start(), 5)
        self.assertIs(type(raised.exception), BacnetError)
        message = str(raised.exception)
        self.assertIn("router port 0", message)
        self.assertIn("bip", message)
        await asyncio.wait_for(router.start(), 5)
        await asyncio.wait_for(router.stop(), 5)

    async def test_start_error_names_sc_port_and_hub(self) -> None:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            unavailable_port = reservation.getsockname()[1]
        url = f"wss://localhost:{unavailable_port}"
        router = BACnetRouter.from_ports(
            [
                RouterBipPort(301, interface="127.0.0.1", port=0),
                RouterScPort(
                    302,
                    url,
                    ROUTER_VMAC,
                    ROUTER_UUID,
                    self.path("site.pem"),
                    self.path("client.pem"),
                    self.path("client.key"),
                ),
            ]
        )
        with self.assertRaises(RuntimeError) as raised:
            await asyncio.wait_for(router.start(), 5)
        self.assertIs(type(raised.exception), RuntimeError)
        message = str(raised.exception)
        self.assertIn("router port 1", message)
        self.assertIn("SC", message)
        self.assertIn(url, message)

    async def test_routed_read_health_routes_and_hub_recovery(self) -> None:
        # A fixed ephemeral selection lets the restarted hub retain the URL used
        # by the router's reconnect connector.
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            hub_port = reservation.getsockname()[1]
        hub = self.hub(listen=f"127.0.0.1:{hub_port}")
        server = BACnetServer(
            3000,
            "SC routed server",
            transport="sc",
            sc_hub=f"wss://localhost:{hub_port}",
            sc_vmac=SERVER_VMAC,
            sc_device_uuid=SERVER_UUID,
            sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"),
        )
        server.add_analog_input(0, "Routed AI", 64, 72.5)
        router = BACnetRouter.from_ports(
            [
                RouterBipPort(
                    BIP_NETWORK,
                    interface="127.0.0.1",
                    port=0,
                    broadcast_address="127.0.0.1",
                ),
                RouterScPort(
                    SC_NETWORK,
                    f"wss://localhost:{hub_port}",
                    ROUTER_VMAC,
                    ROUTER_UUID,
                    self.path("site.pem"),
                    self.path("client.pem"),
                    self.path("client.key"),
                    heartbeat_interval_ms=3000,
                    heartbeat_timeout_ms=3100,
                    reconnect_initial_delay_ms=50,
                    reconnect_max_delay_ms=200,
                    reconnect_forever=True,
                ),
            ]
        )
        client = BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_timeout_ms=3000,
        )
        hub_running = server_running = router_running = False
        try:
            await asyncio.wait_for(hub.start(), 5)
            hub_running = True
            await asyncio.wait_for(server.start(), 5)
            server_running = True
            await asyncio.wait_for(router.start(), 5)
            router_running = True
            router_address = await router.local_address()
            target = RoutedTarget(router_address, SC_NETWORK, SERVER_VMAC)

            async with client:
                value = await asyncio.wait_for(
                    client.read_property(
                        target,
                        ObjectIdentifier(ObjectType.ANALOG_INPUT, 0),
                        PropertyIdentifier.PRESENT_VALUE,
                    ),
                    5,
                )
                self.assertEqual(value.value, 72.5)
                counters = await router.port_counters()
                self.assertGreater(counters[0].forwarded_unicast, 0)
                self.assertGreater(counters[1].forwarded_unicast, 0)
                health = await router.port_health()
                self.assertEqual([item.state for item in health], ["Up", "Up"])
                routes = await router.routing_table()
                self.assertEqual(
                    [(item.network_number, item.port_index) for item in routes],
                    [(BIP_NETWORK, 0), (SC_NETWORK, 1)],
                )
                self.assertTrue(all(item.directly_connected for item in routes))

                await asyncio.wait_for(hub.stop(), 5)
                hub_running = False
                await wait_for_sc_state(router, "Reconnecting")
                await asyncio.wait_for(hub.start(), 5)
                hub_running = True
                await wait_for_sc_state(router, "Up")

                # BACnetServer does not yet configure reconnect; restart its SC
                # attachment after the hub returns, then prove routed traffic.
                await asyncio.wait_for(server.stop(), 5)
                server_running = False
                # Server stop releases its registered object database; restore
                # the fixture object before bringing the SC endpoint back.
                server.add_analog_input(0, "Routed AI", 64, 72.5)
                await asyncio.wait_for(server.start(), 5)
                server_running = True
                recovered = await asyncio.wait_for(
                    client.read_property(
                        target,
                        ObjectIdentifier(ObjectType.ANALOG_INPUT, 0),
                        PropertyIdentifier.PRESENT_VALUE,
                    ),
                    5,
                )
                self.assertEqual(recovered.value, 72.5)
        finally:
            if router_running:
                await asyncio.wait_for(router.stop(), 5)
            if server_running:
                await asyncio.wait_for(server.stop(), 5)
            if hub_running:
                await asyncio.wait_for(hub.stop(), 5)
