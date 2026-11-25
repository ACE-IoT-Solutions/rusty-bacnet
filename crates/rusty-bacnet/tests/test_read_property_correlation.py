"""Installed standalone/endpoint ACK identity and wildcard behavior over B/IP."""
import asyncio
import socket
import unittest

from async_compat import async_timeout, sock_recvfrom, sock_sendto

from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, BacnetDecodingError,
    ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)


class ReadPropertyCorrelationTests(unittest.IsolatedAsyncioTestCase):
    async def test_device_alias_against_bundled_server_both_clients(self):
        server = BACnetServer(9123, interface="127.0.0.1", port=0)
        endpoint = BipEndpoint(device_instance=9124, interface="127.0.0.1", port=0)
        await server.start()
        await endpoint.start()
        try:
            role = await endpoint.client()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                for reader in (role, client):
                    for instance in (4194303, 9123):
                        with self.subTest(reader=type(reader).__name__, instance=instance):
                            async with async_timeout(3):
                                value = await reader.read_property(
                                    await server.local_address(),
                                    ObjectIdentifier(ObjectType.DEVICE, instance),
                                    PropertyIdentifier.OBJECT_IDENTIFIER,
                                )
                            self.assertEqual(value, PropertyValue.object_identifier(
                                ObjectIdentifier(ObjectType.DEVICE, 9123)))
        finally:
            await endpoint.close()
            await server.stop()

    async def test_controlled_peer_identity_rejection_and_network_port_alias(self):
        # Independent UDP vectors: the bundled server's receiving-port alias
        # resolution is a separate issue, not inferred from this client test.
        endpoint = BipEndpoint(device_instance=9125, interface="127.0.0.1", port=0)
        await endpoint.start()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            loop = asyncio.get_running_loop()
            try:
                role = await endpoint.client()
                async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                    for reader in (role, client):
                        for kind, requested, reported_kind, reported, fault in (
                            (ObjectType.DEVICE, 4194303, ObjectType.DEVICE, 10, None),
                            (ObjectType.NETWORK_PORT, 4194303, ObjectType.NETWORK_PORT, 17, None),
                            (ObjectType.DEVICE, 4194303, ObjectType.DEVICE, 4194303, "object"),
                            (ObjectType.DEVICE, 4194303, ObjectType.NETWORK_PORT, 10, "object"),
                            (ObjectType.DEVICE, 10, ObjectType.DEVICE, 11, "object"),
                            (ObjectType.DEVICE, 4194303, ObjectType.DEVICE, 10, "property"),
                            (ObjectType.DEVICE, 4194303, ObjectType.DEVICE, 10, "index"),
                            (ObjectType.DEVICE, 4194303, ObjectType.DEVICE, 10, "malformed"),
                            (ObjectType.DEVICE, 10, ObjectType.DEVICE, 10, None),
                        ):
                            with self.subTest(reader=type(reader).__name__, kind=kind, fault=fault):
                                pending = asyncio.ensure_future(reader.read_property(
                                    address, ObjectIdentifier(kind, requested), PropertyIdentifier.OBJECT_NAME))
                                try:
                                    wire, remote = await asyncio.wait_for(sock_recvfrom(loop, peer, 2048), 2)
                                    self.assertEqual(wire[:2], b"\x81\x0a")
                                    self.assertEqual(wire[4:6], b"\x01\x04")  # expecting reply
                                    self.assertEqual(wire[9], 12)  # confirmed ReadProperty
                                    self.assertEqual(wire[10], 0x0c)
                                    self.assertEqual(int.from_bytes(wire[11:15], "big"),
                                                     (kind.to_raw() << 22) | requested)
                                    body = b"\x0c" + ((reported_kind.to_raw() << 22) | reported).to_bytes(4, "big")
                                    body += bytes([0x19, 28 if fault == "property" else 77])
                                    if fault == "index":
                                        body += b"\x29\x00"
                                    body += b"\x3e\x21\x2a\x3f"
                                    if fault == "malformed":
                                        body = b"\x00"
                                    payload = b"\x01\x00\x30" + bytes([wire[8], 12]) + body
                                    response = b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload
                                    await sock_sendto(loop, peer, response, remote)
                                    if fault:
                                        with self.assertRaises(BacnetDecodingError) as raised:
                                            await asyncio.wait_for(pending, 2)
                                        error = raised.exception
                                        self.assertIsNone(error.read_context)
                                        self.assertEqual(error.offset, 0)
                                        self.assertEqual(error.kind, "invalid_tag" if fault == "malformed"
                                                         else "invalid_encoding")
                                    else:
                                        self.assertEqual(await asyncio.wait_for(pending, 2), PropertyValue.unsigned(42))
                                finally:
                                    if not pending.done():
                                        pending.cancel()
                                    await asyncio.gather(pending, return_exceptions=True)
            finally:
                await endpoint.close()

    async def test_correlated_legacy_event_parameters_preserve_exact_bytes_on_projection_failure(self):
        from test_batch_concurrency import receive
        from test_batch_results import reply
        from test_typed_tags_reads import reader

        # The service decoder accepts this historical opaque representation.
        # Its extended tag is outside the range the generic value reader takes.
        raw = bytes.fromhex("fe ff 01 ff ff 3f 49 08 02 ff ff")
        obj = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, 1)
        prop = PropertyIdentifier.EVENT_PARAMETERS
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            for endpoint in (False, True):
                async with reader(endpoint) as client:
                    await client.add_device(9102, address)
                    for method in ("read_property", "read_property_from_device", "read_property_from_devices"):
                        with self.subTest(endpoint=endpoint, method=method):
                            if method == "read_property_from_devices":
                                operation = client.read_property_from_devices([(9102, obj, prop, None)])
                            elif method == "read_property_from_device":
                                operation = client.read_property_from_device(9102, obj, prop)
                            else:
                                operation = client.read_property(address, obj, prop)
                            wire, remote = await receive(peer)
                            self.assertEqual(wire[9], 12)
                            await reply(peer, remote, bytes((0x30, wire[8], 12)) + wire[10:]
                                        + b"\x3e" + raw + b"\x3f")
                            if method == "read_property_from_devices":
                                row, = await asyncio.wait_for(operation, 2)
                                self.assertIsNone(row["value"])
                                error = row["error"]
                                self.assertIsInstance(error, BacnetDecodingError)
                            else:
                                with self.assertRaises(BacnetDecodingError) as raised:
                                    await asyncio.wait_for(operation, 2)
                                error = raised.exception
                            self.assertEqual((error.kind, error.offset), ("invalid_tag", 1))
                            self.assertIsNone(error.index)
                            self.assertIsNone(error.context)
                            self.assertEqual(error.values, (255, 0))
                            self.assertEqual(str(error), "decoding error at offset 1: invalid extended tag number 255")
                            context = error.read_context
                            self.assertEqual(context.object_id, obj)
                            self.assertEqual(context.property_id, prop.to_raw())
                            self.assertIsNone(context.array_index)
                            self.assertEqual(context.raw, raw)
