"""Installed-wheel tests for the isolated read-only BVLC management probes."""

from __future__ import annotations

import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, BacnetBvlcError, BacnetTimeoutError


class _Responder(asyncio.DatagramProtocol):
    def __init__(self) -> None:
        self.mode = "success"
        self.requests: asyncio.Queue[tuple[int, tuple[str, int]]] = asyncio.Queue()

    def connection_made(self, transport) -> None:
        self.transport = transport

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) != 4 or data[0] != 0x81 or data[2:] != b"\x00\x04":
            return
        function = data[1]
        self.requests.put_nowait((function, address))
        if self.mode == "silent":
            return
        if self.mode == "nak":
            result = 0x0020 if function == 0x02 else 0x0040
            self.transport.sendto(
                b"\x81\x00\x00\x06" + result.to_bytes(2, "big"), address
            )
            return
        if function == 0x02:
            payload = bytes((10, 20, 30, 40)) + (47809).to_bytes(2, "big")
            payload += bytes((255, 255, 255, 0))
            response_function = 0x03
        elif function == 0x06:
            payload = bytes((10, 20, 30, 50)) + (47810).to_bytes(2, "big")
            payload += (120).to_bytes(2, "big") + (143).to_bytes(2, "big")
            response_function = 0x07
        else:
            return
        frame = b"\x81" + bytes((response_function,))
        frame += (len(payload) + 4).to_bytes(2, "big") + payload
        self.transport.sendto(frame, address)


class BvllManagementTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.responder = _Responder()
        self.transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: self.responder, local_addr=("127.0.0.1", 0)
        )
        host, port = self.transport.get_extra_info("sockname")
        self.address = f"{host}:{port}"
        self.client = BACnetClient(
            interface="127.0.0.1", port=0, broadcast_address="127.0.0.1"
        )

    async def asyncTearDown(self) -> None:
        self.transport.close()

    async def assert_probe_socket_released(self) -> None:
        _, source = await asyncio.wait_for(self.responder.requests.get(), 1)
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            probe.bind(source)
        finally:
            probe.close()

    async def test_preserves_all_wire_fields_and_releases_socket(self) -> None:
        bdt = await self.client.read_bdt(self.address, timeout_ms=200)
        await self.assert_probe_socket_released()
        self.assertEqual(
            (bdt[0].ip_bytes, bdt[0].port, bdt[0].broadcast_mask_bytes),
            (bytes((10, 20, 30, 40)), 47809, b"\xff\xff\xff\x00"),
        )

        fdt = await self.client.read_fdt(self.address, timeout_ms=200)
        await self.assert_probe_socket_released()
        self.assertEqual(
            (fdt[0].ip_bytes, fdt[0].port, fdt[0].ttl, fdt[0].seconds_remaining),
            (bytes((10, 20, 30, 50)), 47810, 120, 143),
        )

    async def test_timeout_nak_and_cancellation_are_typed_or_clean(self) -> None:
        self.responder.mode = "silent"
        with self.assertRaises(BacnetTimeoutError):
            await self.client.read_bdt(self.address, timeout_ms=20)
        await self.assert_probe_socket_released()

        self.responder.mode = "nak"
        with self.assertRaises(BacnetBvlcError) as raised:
            await self.client.read_fdt(self.address, timeout_ms=200)
        await self.assert_probe_socket_released()
        self.assertEqual(raised.exception.result_code, 0x0040)

        self.responder.mode = "silent"
        task = asyncio.ensure_future(self.client.read_fdt(self.address, timeout_ms=5000))
        _, source = await asyncio.wait_for(self.responder.requests.get(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        await asyncio.sleep(0.05)
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            probe.bind(source)
        finally:
            probe.close()

    async def test_rejects_non_bip_and_non_unicast_targets(self) -> None:
        with self.assertRaises(ValueError):
            await self.client.read_bdt("255.255.255.255:47808")
        client = BACnetClient(transport="ipv6")
        with self.assertRaisesRegex(Exception, "requires transport='bip'"):
            await client.read_fdt(self.address)


if __name__ == "__main__":
    unittest.main()
