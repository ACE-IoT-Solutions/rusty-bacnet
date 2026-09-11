"""Installed-wheel tests for the opt-in server APDU diagnostic tap."""

from __future__ import annotations

import asyncio
import socket
import unittest

import rusty_bacnet as bacnet


def _bip_original_unicast(npdu: bytes) -> bytes:
    return b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu


class ServerApduObserverInstalledWheelTests(unittest.IsolatedAsyncioTestCase):
    def test_is_off_by_default_and_capacity_requires_opt_in(self) -> None:
        server = bacnet.BACnetServer(device_instance=2201)
        with self.assertRaisesRegex(RuntimeError, "disabled"):
            server.apdu_events()
        with self.assertRaisesRegex(ValueError, "requires apdu_observer=True"):
            bacnet.BACnetServer(device_instance=2202, apdu_observer_capacity=1)
        with self.assertRaisesRegex(ValueError, "greater than zero"):
            bacnet.BACnetServer(
                device_instance=2203,
                apdu_observer=True,
                apdu_observer_capacity=0,
            )

    async def test_startup_safe_decode_recovery_progress_and_clean_restart(self) -> None:
        server = bacnet.BACnetServer(
            device_instance=2210,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_observer=True,
        )
        events = server.apdu_events()
        await server.start()
        target = ("127.0.0.1", int((await server.local_address()).rsplit(":", 1)[1]))
        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            malformed_npdu = b"\x02\x00\x10\x08"
            sender.sendto(_bip_original_unicast(malformed_npdu), target)
            malformed = await asyncio.wait_for(events.__anext__(), 2)
            self.assertEqual(malformed.direction, "inbound")
            self.assertEqual(malformed.raw_npdu, malformed_npdu)
            self.assertFalse(malformed.decoded)
            self.assertEqual(malformed.decode_error.stage, "npdu")

            who_is_npdu = b"\x01\x00\x10\x08"
            sender.sendto(_bip_original_unicast(who_is_npdu), target)
            inbound = await asyncio.wait_for(events.__anext__(), 2)
            outbound = await asyncio.wait_for(events.__anext__(), 2)
            self.assertEqual(inbound.direction, "inbound")
            self.assertEqual(inbound.pdu_type, "unconfirmed_request")
            self.assertEqual(inbound.service_choice, 8)
            self.assertEqual(outbound.direction, "outbound")
            self.assertEqual(outbound.pdu_type, "unconfirmed_request")
            self.assertEqual(outbound.service_choice, 0)
        finally:
            sender.close()
            await server.stop()

        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(events.__anext__(), 1)

        restarted_events = server.apdu_events()
        await server.start()
        target = ("127.0.0.1", int((await server.local_address()).rsplit(":", 1)[1]))
        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            sender.sendto(
                _bip_original_unicast(b"\x02\x00\x10\x08"),
                target,
            )
            restarted = await asyncio.wait_for(restarted_events.__anext__(), 2)
            self.assertEqual(restarted.direction, "inbound")
            self.assertFalse(restarted.decoded)
        finally:
            sender.close()
            await server.stop()

    async def test_bounded_channel_reports_exact_lag(self) -> None:
        server = bacnet.BACnetServer(
            device_instance=2220,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_observer=True,
            apdu_observer_capacity=1,
        )
        events = server.apdu_events()
        await server.start()
        target = ("127.0.0.1", int((await server.local_address()).rsplit(":", 1)[1]))
        sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            for marker in (1, 2, 3):
                sender.sendto(
                    _bip_original_unicast(bytes((0x02, marker, 0x10, 0x08))),
                    target,
                )
            await asyncio.sleep(0.05)
            with self.assertRaises(bacnet.BacnetNotificationLagError) as raised:
                await asyncio.wait_for(events.__anext__(), 2)
            self.assertEqual(raised.exception.skipped, 2)
        finally:
            sender.close()
            await server.stop()


if __name__ == "__main__":
    unittest.main()
