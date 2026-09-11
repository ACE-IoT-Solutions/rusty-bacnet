"""Installed-wheel compatibility tests for managed foreign-device clients."""

from __future__ import annotations

import asyncio
import unittest

from rusty_bacnet import BACnetClient, ForeignDeviceStatus


class _AcceptingBbmd(asyncio.DatagramProtocol):
    def __init__(self) -> None:
        self.registrations: asyncio.Queue[tuple[int, tuple[str, int]]] = asyncio.Queue()

    def connection_made(self, transport: asyncio.DatagramTransport) -> None:
        self.transport = transport

    def datagram_received(self, data: bytes, address: tuple[str, int]) -> None:
        if len(data) != 6 or data[:2] != b"\x81\x05" or data[2:4] != b"\x00\x06":
            return
        ttl = int.from_bytes(data[4:6], "big")
        self.registrations.put_nowait((ttl, address))
        self.transport.sendto(b"\x81\x00\x00\x06\x00\x00", address)


class ForeignDeviceClientCompatibilityTests(unittest.IsolatedAsyncioTestCase):
    def test_configuration_is_paired_bip_only_and_unicast(self) -> None:
        invalid = (
            {"bbmd_address": "127.0.0.1:47808"},
            {"foreign_device_ttl": 30},
            {"bbmd_address": "127.0.0.1:47808", "foreign_device_ttl": 0},
            {
                "transport": "ipv6",
                "bbmd_address": "127.0.0.1:47808",
                "foreign_device_ttl": 30,
            },
            {"bbmd_address": "255.255.255.255:47808", "foreign_device_ttl": 30},
            {"bbmd_address": "224.0.0.1:47808", "foreign_device_ttl": 30},
            {"bbmd_address": "127.0.0.1:0", "foreign_device_ttl": 30},
            {"bbmd_address": "not-an-endpoint", "foreign_device_ttl": 30},
        )
        for kwargs in invalid:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                BACnetClient(**kwargs)

    async def test_status_is_none_when_unconfigured_or_not_started(self) -> None:
        self.assertIsNone(await BACnetClient().foreign_device_status())
        configured = BACnetClient(
            bbmd_address="127.0.0.1:47808", foreign_device_ttl=30
        )
        self.assertIsNone(await configured.foreign_device_status())

    async def test_registration_status_uses_bounded_native_handle(self) -> None:
        loop = asyncio.get_running_loop()
        protocol = _AcceptingBbmd()
        transport, _ = await loop.create_datagram_endpoint(
            lambda: protocol, local_addr=("127.0.0.1", 0)
        )
        host, port = transport.get_extra_info("sockname")
        client = BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            bbmd_address=f"{host}:{port}",
            foreign_device_ttl=30,
        )
        try:
            async with client:
                ttl, _source = await asyncio.wait_for(protocol.registrations.get(), 2)
                self.assertEqual(ttl, 30)
                for _ in range(100):
                    status = await client.foreign_device_status()
                    if status is not None and status.state == "registered":
                        break
                    await asyncio.sleep(0.01)
                else:
                    self.fail("foreign-device registration did not become registered")

                self.assertIsInstance(status, ForeignDeviceStatus)
                self.assertEqual(status.last_result, 0)
                self.assertIsNotNone(status.seconds_to_renewal)

            stopped = await client.foreign_device_status()
            self.assertIsInstance(stopped, ForeignDeviceStatus)
        finally:
            transport.close()


if __name__ == "__main__":
    unittest.main()
