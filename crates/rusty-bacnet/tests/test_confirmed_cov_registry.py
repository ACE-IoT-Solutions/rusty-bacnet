"""Confirmed COV admission on a standalone client and a shared endpoint seat."""

import asyncio
import socket
import unittest

import rusty_bacnet as rb
from async_compat import sock_recvfrom
from bvll_peer import mac
from test_endpoint_routed import request_of
from test_routed_cov import AI_1, FAR, STATION, notification, relay


class ConfirmedCovRegistryTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(self.sock.close)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.setblocking(False)
        self.peer = "%s:%d" % self.sock.getsockname()
        self.kwargs = dict(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")

    async def receive(self):
        return await asyncio.wait_for(sock_recvfrom(asyncio.get_running_loop(), self.sock, 2048), 5)

    async def send_confirmed(self, to, invoke, process, network=None):
        context = notification(process, 17.0)[2:]
        pdu = bytes((0, 5, invoke, 1)) + context
        self.sock.sendto(relay(network, STATION, pdu), to)
        datagram, source = await self.receive()
        self.assertEqual(source, to)
        reply = request_of(datagram)
        self.assertEqual(reply["dnet"], network)
        self.assertEqual(reply["dadr"], None if network is None else STATION)
        return reply["apdu"]

    async def strict_exchange(self, client, to, target, network):
        iterator = await client.cov_notifications()
        lease = await client.expect_confirmed_cov(
            target, 7, AI_1, initiating_device_instance=FAR)
        self.assertIsInstance(lease, rb.ConfirmedCovLease)
        with self.assertRaisesRegex(rb.BacnetError, "already registered"):
            await client.expect_confirmed_cov(target, 7, AI_1, initiating_device_instance=FAR)
        with self.assertRaisesRegex(rb.BacnetError, "registry is full"):
            await client.expect_confirmed_cov(target, 9, AI_1)
        self.assertEqual(await self.send_confirmed(to, 17, 9, network), bytes.fromhex("50 11 01 91 05 91 4f"))
        self.assertEqual(await self.send_confirmed(to, 18, 7, network), bytes.fromhex("20 12 01"))
        event = await asyncio.wait_for(anext(iterator), 5)
        self.assertEqual(event.subscriber_process_identifier, 7)
        lease.close()
        lease.close()
        self.assertEqual(await self.send_confirmed(to, 19, 7, network), bytes.fromhex("50 13 01 91 05 91 4f"))
        return lease

    async def test_standalone_registry(self):
        async with rb.BACnetClient(**self.kwargs, confirmed_cov_subscription_limit=1) as client:
            await client.who_is(scope=self.peer)
            _, to = await self.receive()
            await self.strict_exchange(client, to, self.peer, None)

    async def test_endpoint_registry_and_new_seat(self):
        async with rb.BipEndpoint(**self.kwargs, device_instance=100, vendor_id=42) as endpoint:
            ip, port = (await endpoint.local_address()).rsplit(":", 1)
            to = (ip, int(port))
            first = await endpoint.client(confirmed_cov_subscription_limit=1)
            self.assertIs(first, await endpoint.client(confirmed_cov_subscription_limit=1))
            with self.assertRaisesRegex(ValueError, "differs"):
                await endpoint.client()
            # Registering a routed source without a known router needs no discovery.
            target = rb.RoutedTarget(50, STATION)
            await self.strict_exchange(first, to, target, 50)
            outstanding = await first.expect_confirmed_cov(target, 7, AI_1)
            await first.stop()
            second = await endpoint.client(confirmed_cov_subscription_limit=1)
            self.assertEqual(await self.send_confirmed(to, 20, 7, 50), bytes.fromhex("50 14 01 91 05 91 4f"))
            current = await second.expect_confirmed_cov(target, 7, AI_1)
            outstanding.close()
            self.assertEqual(await self.send_confirmed(to, 21, 7, 50), bytes.fromhex("20 15 01"))
            current.close()

    async def test_own_network_target_accepts_a_direct_notification(self):
        async with rb.BipEndpoint(device_instance=100, vendor_id=42, interface="127.0.0.1",
                                  port=0, broadcast_address="127.255.255.255",
                                  network_number=50, registered_network_port=1) as endpoint:
            ip, port = (await endpoint.local_address()).rsplit(":", 1)
            client = await endpoint.client(confirmed_cov_subscription_limit=1)
            target = rb.RoutedTarget(50, mac(self.peer), router="127.0.0.1:1")
            await self.strict_exchange(client, (ip, int(port)), target, None)

    async def test_default_seat_acknowledges_without_registration(self):
        async with rb.BipEndpoint(**self.kwargs, device_instance=100, vendor_id=42) as endpoint:
            ip, port = (await endpoint.local_address()).rsplit(":", 1)
            client = await endpoint.client()
            with self.assertRaisesRegex(rb.BacnetError, "registry is disabled"):
                await client.expect_confirmed_cov(self.peer, 7, AI_1)
            self.assertEqual(await self.send_confirmed((ip, int(port)), 22, 9), bytes.fromhex("20 16 01"))
