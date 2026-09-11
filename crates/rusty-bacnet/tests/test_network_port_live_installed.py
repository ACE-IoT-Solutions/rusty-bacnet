"""Installed-wheel acceptance for live B/IP NetworkPort publication."""

from __future__ import annotations

import asyncio
import socket
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetBbmdControlError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)


DEVICE_INSTANCE = 811_009
NETWORK_PORT_INSTANCE = 1


def _bdt_row(ip: str, port: int, mask: str) -> bytes:
    return socket.inet_aton(ip) + port.to_bytes(2, "big") + socket.inet_aton(mask)


async def _read_value(
    client: BACnetClient,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
):
    return (await client.read_property(address, object_id, property_id)).value


async def _wait_for_value(read, predicate, description: str, timeout: float = 3.0):
    deadline = asyncio.get_running_loop().time() + timeout
    last = None
    while asyncio.get_running_loop().time() < deadline:
        last = await read()
        if predicate(last):
            return last
        await asyncio.sleep(0.05)
    raise AssertionError(f"timed out waiting for {description}; last value was {last!r}")


class InstalledNetworkPortLiveTests(unittest.IsolatedAsyncioTestCase):
    async def test_any_transport_publishes_live_bbmd_network_port(self) -> None:
        server = BACnetServer(
            DEVICE_INSTANCE,
            "Live NetworkPort Device",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            bbmd=True,
            bbmd_bdt=[("192.0.2.17", 47808, "255.255.255.255")],
            bbmd_accept_foreign_devices=True,
        )
        # NetworkType.IPV4 is raw value 5. The current binding intentionally
        # retains raw integer enum input for object registration.
        server.add_network_port(
            NETWORK_PORT_INSTANCE, "Primary B/IP NetworkPort", network_type=5
        )
        control = server.bbmd_control
        await server.start()
        address = await server.local_address()
        host, port_text = address.rsplit(":", 1)
        actual_port = int(port_text)
        self.assertNotEqual(actual_port, 0)

        device = ObjectIdentifier(ObjectType.DEVICE, DEVICE_INSTANCE)
        network_port = ObjectIdentifier(ObjectType.NETWORK_PORT, NETWORK_PORT_INSTANCE)
        observer = BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_timeout_ms=2_000,
        )
        foreign = BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            bbmd_address=address,
            foreign_device_ttl=8,
        )

        try:
            async with observer, foreign:
                status = await _wait_for_value(
                    foreign.foreign_device_status,
                    lambda current: current is not None
                    and current.state == "registered",
                    "managed foreign-device registration",
                )
                self.assertEqual(status.last_result, 0)

                self.assertEqual(
                    await _read_value(
                        observer, address, device, PropertyIdentifier.OBJECT_NAME
                    ),
                    "Live NetworkPort Device",
                )
                self.assertEqual(
                    await _read_value(
                        observer, address, network_port, PropertyIdentifier.NETWORK_TYPE
                    ),
                    5,
                )
                self.assertEqual(
                    await _read_value(
                        observer, address, network_port, PropertyIdentifier.IP_ADDRESS
                    ),
                    socket.inet_aton(host),
                )
                self.assertEqual(
                    await _read_value(
                        observer,
                        address,
                        network_port,
                        PropertyIdentifier.BACNET_IP_UDP_PORT,
                    ),
                    actual_port,
                )
                self.assertEqual(
                    await _read_value(
                        observer, address, network_port, PropertyIdentifier.MAC_ADDRESS
                    ),
                    socket.inet_aton(host) + actual_port.to_bytes(2, "big"),
                )
                self.assertEqual(
                    await _read_value(
                        observer,
                        address,
                        network_port,
                        PropertyIdentifier.BACNET_IP_MODE,
                    ),
                    2,
                )

                async def read_fdt():
                    return await _read_value(
                        observer,
                        address,
                        network_port,
                        PropertyIdentifier.BBMD_FOREIGN_DEVICE_TABLE,
                    )

                # The server-owned publisher uses a bounded one-second cadence;
                # wait for one complete refresh after native registration.
                await asyncio.sleep(1.1)
                fdt = await read_fdt()
                self.assertEqual(len(fdt), 1)
                self.assertEqual(len(fdt[0]), 10)
                self.assertEqual(fdt[0][:4], socket.inet_aton("127.0.0.1"))
                self.assertEqual(int.from_bytes(fdt[0][6:8], "big"), 8)
                self.assertGreater(int.from_bytes(fdt[0][8:10], "big"), 0)

                replacement = _bdt_row(
                    "198.51.100.23", 47809, "255.255.255.255"
                )
                revision = await control.replace_bdt(
                    [("198.51.100.23", 47809, "255.255.255.255")]
                )
                await control.set_accept_foreign_devices(False)
                self.assertGreaterEqual(revision, 1)

                async def read_bdt():
                    return await _read_value(
                        observer,
                        address,
                        network_port,
                        PropertyIdentifier.BBMD_BROADCAST_DISTRIBUTION_TABLE,
                    )

                bdt = await _wait_for_value(
                    read_bdt,
                    lambda rows: replacement in rows,
                    "controller-updated BDT",
                )
                self.assertIn(replacement, bdt)

                async def read_accept():
                    return await _read_value(
                        observer,
                        address,
                        network_port,
                        PropertyIdentifier.BBMD_ACCEPT_FD_REGISTRATIONS,
                    )

                self.assertFalse(
                    await _wait_for_value(
                        read_accept,
                        lambda accepted: accepted is False,
                        "controller-updated FD acceptance",
                    )
                )
                snapshot = await control.snapshot()
                self.assertFalse(snapshot.accept_foreign_devices)
                self.assertEqual(snapshot.bdt[0].ip, "198.51.100.23")
        finally:
            await server.stop()

        self.assertEqual(control.lifecycle, "stopped")
        with self.assertRaises(BacnetBbmdControlError):
            await control.snapshot()
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await server.read_property(network_port, PropertyIdentifier.IP_ADDRESS)

        # The native publisher is cleared during stop and the B/IP socket is
        # released before the await completes. The Python facade then fails
        # closed instead of exposing a stale live snapshot.
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            probe.bind((host, actual_port))
        finally:
            probe.close()


if __name__ == "__main__":
    unittest.main()
