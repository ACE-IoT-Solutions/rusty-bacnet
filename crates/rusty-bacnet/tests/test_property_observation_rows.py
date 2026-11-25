"""Original COV value octets and per-selector RPM diagnostics over B/IP."""

import asyncio
import socket
import unittest

import rusty_bacnet as rb
from async_compat import async_timeout, sock_recvfrom, sock_sendto
from test_endpoint_routed import request_of
from test_routed_cov import STATION, relay
from test_typed_tags_reads import reader

P = rb.PropertyIdentifier
EE = rb.ObjectIdentifier(rb.ObjectType.EVENT_ENROLLMENT, 1)
UCS4_OMEGA = bytes.fromhex("75 05 03 00 00 03 a9")
UTF8_OMEGA = bytes.fromhex("73 00 ce a9")
LEGACY = bytes.fromhex("fe ff 01 ff ff 4f 02 ff ff")
# Event Enrollment 1 returns the accepted legacy Event_Parameters form, a
# Notification_Class value, an unknown-property error, and an opaque string.
RPM_BODY = (bytes.fromhex("0c 02400001 1e 29 53 4e") + LEGACY
            + bytes.fromhex("4f 29 11 4e 21 08 4f 29 4d 5e 91 02 91 20 5f 29 1c 4e 72 00 ff 4f 1f"))
# Device 1 Object_List[0] with a two-octet unsigned 7. Three proprietary
# properties report Null, an empty OCTET STRING and an opaque string. Description is UCS-4.
VALUES = bytes.fromhex("09 4c 19 00 2e 22 0007 2f 0a 0200 2e 00 2f 0a 0201 2e 60 2f 09 1c 2e") + UCS4_OMEGA + bytes.fromhex("2f 0a 0202 2e 72 00 ff 2f")
SINGLE = bytes.fromhex("10 02 09 07 1c 02000001 2c 02000001 39 3c 4e") + VALUES + b"\x4f"
MULTIPLE = (bytes.fromhex("10 0b 09 07 1c 02000001 29 3c 3e a4 7e 0a 05 01 b4 09 00 00 00 3f"
                          " 4e 0c 02000001 1e") + VALUES + bytes.fromhex("3c 09 00 00 00 1f 4f"))


class PropertyObservationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(self.peer.close)
        self.peer.bind(("127.0.0.1", 0))
        self.peer.setblocking(False)
        self.address = "%s:%d" % self.peer.getsockname()

    async def receive(self):
        return await sock_recvfrom(asyncio.get_running_loop(), self.peer, 2048)

    async def send(self, to, network, apdu):
        await sock_sendto(asyncio.get_running_loop(), self.peer, relay(network, STATION, apdu), to)

    async def test_single_and_multiple_cov_keep_received_octets(self):
        for endpoint in (False, True):
            async with reader(endpoint) as client, async_timeout(30):
                events = await client.cov_notifications()
                await client.who_is(scope=self.address)
                _, to = await self.receive()
                for apdu, network in ((SINGLE, None), (MULTIPLE, 5)):
                    with self.subTest(endpoint=endpoint, network=network):
                        await self.send(to, network, apdu)
                        event = await anext(events)
                        self.assertEqual(event.source_network, network)
                        if network is None:
                            self.assertEqual(event.kind, "object")
                            values = event.values
                        else:
                            self.assertEqual(event.kind, "multiple")
                            obj, = event.objects
                            values = obj["values"]
                            self.assertEqual([row["time_of_change"] for row in values],
                                             [None, None, None, None, (9, 0, 0, 0)])
                        self.assertEqual([row["array_index"] for row in values], [0, None, None, None, None])
                        self.assertEqual([row["property_id"].to_raw() for row in values], [76, 512, 513, 28, 514])
                        number, null, empty, text, opaque = [row["value"] for row in values]
                        self.assertEqual(number, rb.PropertyValue.unsigned(7))
                        self.assertEqual(number.octets, bytes.fromhex("21 07"))
                        self.assertEqual(null, rb.PropertyValue.null())
                        self.assertEqual(empty, rb.PropertyValue.octet_string(b""))
                        self.assertEqual(text, rb.PropertyValue.character_string("Ω"))
                        self.assertEqual(text.octets, UTF8_OMEGA)
                        self.assertEqual(opaque, rb.PropertyValue.application_data(bytes.fromhex("72 00 ff")))
                        self.assertTrue(all(row["value"].read_context is None for row in values))
                        self.assertEqual([row["raw"] for row in values],
                                         [bytes.fromhex("22 0007"), b"\x00", b"\x60", UCS4_OMEGA, bytes.fromhex("72 00 ff")])
                        # Index zero of an ObjectIdentifier array is its unsigned count.
                        observed = values[0]
                        declared = rb.decode_property_as(observed["raw"], "ObjectIdentifier",
                                                         shape="array", array_index=observed["array_index"])
                        self.assertEqual((declared.status, declared.value, declared.raw),
                                         ("decoded", number, bytes.fromhex("22 00 07")))

    async def test_ucs4_reads_keep_original_value_and_write_utf8(self):
        identity = bytes.fromhex("0c 02400001")
        for endpoint in (False, True):
            async with reader(endpoint) as client, async_timeout(30):
                for network in (None, 5):
                    target = self.address if network is None else rb.RoutedTarget(5, STATION, router=self.address)
                    for rpm in (False, True):
                        with self.subTest(endpoint=endpoint, network=network, rpm=rpm):
                            if rpm:
                                call = client.read_property_multiple(target, [(EE, [(P.DESCRIPTION, None)])])
                                body = identity + bytes.fromhex("1e 29 1c 4e") + UCS4_OMEGA + bytes.fromhex("4f 1f")
                            else:
                                call = client.read_property(target, EE, P.DESCRIPTION)
                                body = identity + bytes.fromhex("19 1c 3e") + UCS4_OMEGA + b"\x3f"
                            wire, to = await self.receive()
                            frame = request_of(wire)
                            self.assertEqual((frame["dnet"], frame["dadr"]),
                                             (network, None if network is None else STATION))
                            request = frame["apdu"]
                            self.assertEqual(request[3], 14 if rpm else 12)
                            await self.send(to, network, bytes((0x30, request[2], request[3])) + body)
                            result = await call
                            if rpm:
                                (obj,) = result
                                (row,) = obj["results"]
                                self.assertIsNone(row["decode_error"])
                                self.assertIsNone(row["error"])
                                result = row["value"]
                            self.assertEqual(result, rb.PropertyValue.character_string("Ω"))
                            context = result.read_context
                            self.assertEqual((context.object_id, context.property_id, context.array_index),
                                             (EE, P.DESCRIPTION.to_raw(), None))
                            self.assertEqual((context.raw, result.octets), (UCS4_OMEGA, UTF8_OMEGA))
                            projected = rb.decode_property_as(context.raw, "CharacterString")
                            self.assertEqual((projected.status, projected.raw, projected.value),
                                             ("decoded", UCS4_OMEGA, result))

                    written = client.write_property(target, EE, P.DESCRIPTION, result)
                    wire, to = await self.receive()
                    request = request_of(wire)["apdu"]
                    await self.send(to, network, bytes((0x20, request[2], 15)))
                    await written
                    self.assertEqual((request[3], request[4:]),
                                     (15, identity + bytes.fromhex("19 1c 3e") + UTF8_OMEGA + b"\x3f"))

    async def test_rpm_local_fault_preserves_sibling_rows_and_returned_selector(self):
        for endpoint in (False, True):
            async with reader(endpoint) as client, async_timeout(30):
                for network in (None, 5):
                    with self.subTest(endpoint=endpoint, network=network):
                        target = self.address if network is None else rb.RoutedTarget(5, STATION, router=self.address)
                        # ALL asks for a selector. The four concrete properties are supplied by the peer.
                        pending = client.read_property_multiple(target, [(EE, [(P.ALL, None)])])
                        wire, to = await self.receive()
                        frame = request_of(wire)
                        self.assertEqual((frame["dnet"], frame["dadr"]),
                                         (network, None if network is None else STATION))
                        request = frame["apdu"]
                        self.assertEqual(request[3], 14)
                        await self.send(to, network, bytes((0x30, request[2], 14)) + RPM_BODY)
                        obj, = await pending
                        self.assertEqual(obj["object_id"], EE)
                        broken, count, remote, opaque = obj["results"]
                        self.assertEqual([row["property_id"] for row in obj["results"]],
                                         [P.EVENT_PARAMETERS, P.NOTIFICATION_CLASS, P.OBJECT_NAME, P.DESCRIPTION])
                        self.assertEqual(broken["value"], LEGACY)
                        self.assertIsNone(broken["error"])
                        self.assertEqual(count["value"], rb.PropertyValue.unsigned(8))
                        self.assertEqual(count["value"].read_context.raw, bytes.fromhex("21 08"))
                        self.assertIsNone(remote["value"])
                        self.assertEqual(tuple(map(int, remote["error"])), (2, 32))
                        self.assertEqual(opaque["value"], rb.PropertyValue.application_data(bytes.fromhex("72 00 ff")))
                        self.assertTrue(all(row["decode_error"] is None for row in (count, remote, opaque)))
                        error = broken["decode_error"]
                        self.assertIs(type(error), rb.BacnetDecodingError)
                        self.assertEqual((error.kind, error.offset, error.values), ("invalid_tag", 1, (255, 0)))
                        context = error.read_context
                        self.assertEqual((context.object_id, context.property_id, context.array_index, context.raw),
                                         (EE, P.EVENT_PARAMETERS.to_raw(), None, LEGACY))
