"""Installed-wheel coverage for configurable BACnet Device identity."""

from __future__ import annotations

import asyncio
import inspect
import socket
import unittest

import rusty_bacnet as bacnet


class DeviceIdentityTests(unittest.TestCase):
    def test_constructor_identity_defaults(self) -> None:
        parameters = inspect.signature(bacnet.BACnetServer).parameters
        expected = {
            "vendor_name": "Rusty BACnet",
            "vendor_identifier": 555,
            "model_name": "rusty-bacnet",
            "description": "",
            "firmware_revision": "0.1.0",
            "application_software_version": "0.1.0",
        }
        for name, default in expected.items():
            self.assertEqual(parameters[name].default, default)

    def test_custom_identity_property_readback_and_i_am_vendor(self) -> None:
        async def run() -> None:
            server = bacnet.BACnetServer(
                4_180_040,
                device_name="Identity Device",
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                vendor_name="Example Controls",
                vendor_identifier=4242,
                model_name="EC-9000",
                description="North plant controller",
                firmware_revision="fw-7.8.9",
                application_software_version="app-10.11.12",
            )
            await server.start()
            try:
                device = bacnet.ObjectIdentifier(bacnet.ObjectType.DEVICE, 4_180_040)
                expected = {
                    bacnet.PropertyIdentifier.OBJECT_NAME: "Identity Device",
                    bacnet.PropertyIdentifier.VENDOR_NAME: "Example Controls",
                    bacnet.PropertyIdentifier.VENDOR_IDENTIFIER: 4242,
                    bacnet.PropertyIdentifier.MODEL_NAME: "EC-9000",
                    bacnet.PropertyIdentifier.DESCRIPTION: "North plant controller",
                    bacnet.PropertyIdentifier.FIRMWARE_REVISION: "fw-7.8.9",
                    bacnet.PropertyIdentifier.APPLICATION_SOFTWARE_VERSION: "app-10.11.12",
                    # Upstream currently owns and validates revision 22.
                    bacnet.PropertyIdentifier.PROTOCOL_REVISION: 22,
                }
                for property_id, expected_value in expected.items():
                    value = await server.read_property(device, property_id)
                    self.assertEqual(value.value, expected_value)

                host, port_text = (await server.local_address()).rsplit(":", 1)
                probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    probe.bind(("127.0.0.1", 0))
                    probe.setblocking(False)
                    # A routed Who-Is makes the response unicast to the immediate
                    # requester, avoiding platform-specific broadcast delivery.
                    npdu = b"\x01\x08\x00\x01\x01\x01\x10\x08"
                    frame = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                    loop = asyncio.get_running_loop()
                    await loop.sock_sendto(probe, frame, (host, int(port_text)))
                    response, _ = await asyncio.wait_for(
                        loop.sock_recvfrom(probe, 2048), timeout=2
                    )
                finally:
                    probe.close()
                self.assertEqual(_decode_i_am_vendor(response), 4242)
            finally:
                await server.stop()

        asyncio.run(run())


def _decode_i_am_vendor(frame: bytes) -> int:
    """Decode Vendor ID from a BVLC/NPDU/Unconfirmed-I-Am response."""
    if frame[:2] != b"\x81\x0a":
        raise AssertionError(f"unexpected BVLC header: {frame[:2]!r}")
    offset = 4
    if frame[offset] != 1:
        raise AssertionError("unexpected NPDU version")
    control = frame[offset + 1]
    offset += 2
    if control & 0x20:
        destination_length = frame[offset + 2]
        offset += 3 + destination_length + 1
    if control & 0x08:
        source_length = frame[offset + 2]
        offset += 3 + source_length
    if frame[offset : offset + 2] != b"\x10\x00":
        raise AssertionError("response is not an unconfirmed I-Am")
    offset += 2

    def application_value(expected_tag: int) -> int:
        nonlocal offset
        header = frame[offset]
        offset += 1
        if header >> 4 != expected_tag:
            raise AssertionError(f"expected application tag {expected_tag}")
        length = header & 0x07
        value = int.from_bytes(frame[offset : offset + length], "big")
        offset += length
        return value

    application_value(12)
    application_value(2)
    application_value(9)
    return application_value(2)


if __name__ == "__main__":
    unittest.main()
