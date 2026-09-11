"""Installed-wheel tests for the opt-in client APDU diagnostic tap."""

from __future__ import annotations

import asyncio
import socket
import unittest

import rusty_bacnet as bacnet


def _bip_original_unicast(npdu: bytes) -> bytes:
    return b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu


def _unused_udp_port() -> int:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])
    finally:
        sock.close()


class ClientApduObserverInstalledWheelTests(unittest.IsolatedAsyncioTestCase):
    def test_is_off_by_default_and_capacity_requires_opt_in(self) -> None:
        client = bacnet.BACnetClient()
        with self.assertRaisesRegex(RuntimeError, "disabled"):
            client.apdu_events()
        with self.assertRaisesRegex(ValueError, "requires apdu_observer=True"):
            bacnet.BACnetClient(apdu_observer_capacity=1)
        with self.assertRaisesRegex(ValueError, "greater than zero"):
            bacnet.BACnetClient(apdu_observer=True, apdu_observer_capacity=0)

    async def test_decode_error_recovery_frozen_event_and_clean_restart(self) -> None:
        port = _unused_udp_port()
        client = bacnet.BACnetClient(
            interface="127.0.0.1",
            port=port,
            broadcast_address="127.0.0.1",
            apdu_observer=True,
        )
        events = client.apdu_events()
        await client.__aenter__()
        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            malformed_npdu = b"\x02\x00\x10\x08"
            sender.sendto(_bip_original_unicast(malformed_npdu), ("127.0.0.1", port))
            malformed = await asyncio.wait_for(events.__anext__(), 2)
            self.assertEqual(malformed.direction, "inbound")
            self.assertEqual(malformed.raw_npdu, malformed_npdu)
            self.assertFalse(malformed.decoded)
            self.assertEqual(malformed.decode_error.stage, "npdu")
            self.assertTrue(malformed.decode_error.message)
            with self.assertRaises(AttributeError):
                malformed.direction = "outbound"

            valid_npdu = b"\x01\x00\x10\x08"
            sender.sendto(_bip_original_unicast(valid_npdu), ("127.0.0.1", port))
            valid = await asyncio.wait_for(events.__anext__(), 2)
            self.assertTrue(valid.decoded)
            self.assertEqual(valid.pdu_type, "unconfirmed_request")
            self.assertEqual(valid.service_choice, 8)
            self.assertIsNone(valid.invoke_id)
            self.assertIsNone(valid.decode_error)
        finally:
            sender.close()
            await client.stop()

        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(events.__anext__(), 1)

        restarted_events = client.apdu_events()
        await client.__aenter__()
        try:
            await client.who_is()
            outbound = await asyncio.wait_for(restarted_events.__anext__(), 2)
            self.assertEqual(outbound.direction, "outbound")
            self.assertTrue(outbound.decoded)
            self.assertEqual(outbound.pdu_type, "unconfirmed_request")
            self.assertEqual(outbound.service_choice, 8)
        finally:
            await client.stop()

    async def test_bounded_channel_reports_exact_lag(self) -> None:
        client = bacnet.BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_observer=True,
            apdu_observer_capacity=1,
        )
        events = client.apdu_events()
        await client.__aenter__()
        try:
            for _ in range(3):
                await client.who_is()
            with self.assertRaises(bacnet.BacnetNotificationLagError) as raised:
                await asyncio.wait_for(events.__anext__(), 2)
            self.assertEqual(raised.exception.skipped, 2)
        finally:
            await client.stop()


if __name__ == "__main__":
    unittest.main()
