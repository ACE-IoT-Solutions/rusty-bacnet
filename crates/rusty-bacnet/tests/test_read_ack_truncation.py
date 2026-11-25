"""Whole service ACKs cut short in transit, through both Python client owners."""

import asyncio
import socket
import unittest

import rusty_bacnet as rb
from async_compat import async_timeout, sock_recvfrom, sock_sendto
from test_endpoint_routed import request_of
from test_routed_cov import STATION, relay
from test_typed_tags_reads import reader

P = rb.PropertyIdentifier
MULTI = rb.ObjectIdentifier(rb.ObjectType.MULTI_STATE_VALUE, 1)
TREND = rb.ObjectIdentifier(rb.ObjectType.TREND_LOG, 1)
FILE = rb.ObjectIdentifier(rb.ObjectType.FILE, 1)


class ReadAckTruncationTests(unittest.IsolatedAsyncioTestCase):
    async def test_correlated_truncated_ack_reports_first_missing_service_byte(self):
        cases = (
            (12, "ReadProperty ACK", "0c0000", 5,
             lambda c, t: c.read_property(t, MULTI, P.PRESENT_VALUE)),
            # A complete Present_Value precedes the truncated Description error.
            (14, "ReadPropertyMultiple ACK", "0c04c000011e29554e21074f291c5e9200", 18,
             lambda c, t: c.read_property_multiple(t, [(MULTI, [(P.PRESENT_VALUE, None), (P.DESCRIPTION, None)])])),
            (26, "ReadRange ACK", "0c0500000119833a05", 10, lambda c, t: c.read_range(t, TREND, P.LOG_BUFFER)),
            (6, "AtomicReadFile ACK", "110e310063aa", 8,
             lambda c, t: c.atomic_read_file(t, FILE, "stream", requested_octet_count=3)),
            (6, "AtomicReadFile ACK", "111e3100210163aa", 10,
             lambda c, t: c.atomic_read_file(t, FILE, "record", requested_record_count=1)),
        )
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = "%s:%d" % peer.getsockname()
            loop = asyncio.get_running_loop()
            for endpoint in (False, True):
                async with reader(endpoint) as client, async_timeout(30):
                    for network in (None, 5):
                        target = address if network is None else rb.RoutedTarget(network, STATION, router=address)
                        for service, context, encoded, need, call in cases:
                            with self.subTest(endpoint=endpoint, network=network, encoded=encoded):
                                data = bytes.fromhex(encoded)
                                pending = call(client, target)
                                wire, sender = await sock_recvfrom(loop, peer, 2048)
                                frame = request_of(wire)
                                self.assertEqual((frame["dnet"], frame["dadr"]),
                                                 (network, None if network is None else STATION))
                                request = frame["apdu"]
                                self.assertEqual(request[3], service)
                                ack = bytes((0x30, request[2], service)) + data
                                await sock_sendto(loop, peer, relay(network, STATION, ack), sender)
                                with self.assertRaises(rb.BacnetDecodingError) as raised:
                                    await pending
                                error = raised.exception
                                self.assertEqual((error.kind, error.offset, error.context, error.values),
                                                 ("missing", len(data), context, (need, len(data))))
                                self.assertIsNone(error.index)
                                self.assertIsNone(error.read_context)
