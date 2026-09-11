"""Installed-wheel tests for duplicate-preserving streaming I-Am provenance."""

from __future__ import annotations

import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, IAmEvent, IAmEventIterator


def _i_am_frame(device_instance: int, vendor_id: int = 15) -> bytes:
    device_oid = (8 << 22) | device_instance
    service_request = (
        b"\xc4"
        + device_oid.to_bytes(4, "big")
        + b"\x22\x05\xc4"  # max APDU 1476
        + b"\x91\x03"  # Segmentation.NONE
        + b"\x21"
        + bytes((vendor_id,))
    )
    npdu = b"\x01\x00" + b"\x10\x00" + service_request
    return b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu


def _forwarded_i_am_frame(
    device_instance: int, origin_ip: str, origin_port: int, vendor_id: int = 15
) -> bytes:
    npdu = _i_am_frame(device_instance, vendor_id)[4:]
    payload = socket.inet_aton(origin_ip) + origin_port.to_bytes(2, "big") + npdu
    return b"\x81\x04" + (len(payload) + 4).to_bytes(2, "big") + payload


class IAmProvenanceTests(unittest.IsolatedAsyncioTestCase):
    async def test_stream_preserves_duplicate_immediate_and_forwarded_sources(self) -> None:
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        probe.bind(("127.0.0.1", 0))
        client_port = probe.getsockname()[1]
        probe.close()

        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sender.bind(("127.0.0.1", 0))
        sender_port = sender.getsockname()[1]
        bbmd = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        bbmd.bind(("127.0.0.1", 0))
        bbmd_port = bbmd.getsockname()[1]
        destination = ("127.0.0.1", client_port)
        instance = 4_190_123

        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=client_port,
                broadcast_address="127.0.0.1",
            ) as client:
                events = await client.who_is_stream()
                self.assertIsInstance(events, IAmEventIterator)

                sender.sendto(_i_am_frame(instance, 37), destination)
                first = await asyncio.wait_for(events.__anext__(), timeout=3)
                bbmd.sendto(
                    _forwarded_i_am_frame(instance, "127.0.0.1", sender_port, 37),
                    destination,
                )
                second = await asyncio.wait_for(events.__anext__(), timeout=3)

                self.assertIsInstance(first, IAmEvent)
                self.assertEqual(first.device_instance, instance)
                self.assertEqual(second.device_instance, instance)
                self.assertIsNot(first, second)
                self.assertEqual(first.udp_source, f"127.0.0.1:{sender_port}")
                self.assertEqual(first.bvlc_function, 0x0A)
                self.assertIsNone(first.forwarded_from)
                self.assertEqual(second.udp_source, f"127.0.0.1:{bbmd_port}")
                self.assertEqual(second.bvlc_function, 0x04)
                self.assertEqual(second.forwarded_from, f"127.0.0.1:{sender_port}")
                self.assertEqual(second.source_mac, first.source_mac)
                self.assertEqual(first.raw_mac, first.source_mac)
                self.assertIsNone(first.snet)
                self.assertIsNone(first.sadr)
                self.assertLessEqual(first.timestamp, asyncio.get_running_loop().time())
                with self.assertRaises(AttributeError):
                    first.vendor_id = 99

                closing_events = await client.who_is_stream()

            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(closing_events.__anext__(), timeout=3)
        finally:
            sender.close()
            bbmd.close()


if __name__ == "__main__":
    unittest.main()
