"""Routed reads and confirmed COV sharing a B/IP endpoint and invoke ID."""

import asyncio
import socket
import unittest

import rusty_bacnet as rb
from async_compat import async_timeout, sock_recvfrom, sock_sendto
from bvll_peer import mac
from test_endpoint_routed import request_of
from test_read_log import TREND_LOG, TREND_RECORD, wrapped_page_ack
from test_routed_cov import AI_1, FAR, PROCESS, STATION, notification, relay
from test_routed_file_services import FILE_1, READ_FILE, parse_read, segments, stream_ack

RP, RPM, READ_RANGE = 12, 14, 26
NETWORK = 5
READ_NETWORK = {READ_FILE: NETWORK, RP: 6, RPM: 7, READ_RANGE: 8}
DEVICE = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 10)
P = rb.PropertyIdentifier
# Device 10 Object_Name and Object_List[0]. The second result is a property error.
RPM_REQUEST = bytes.fromhex("0c 0200000a 1e 09 4d 09 4c 19 00 1f")
RPM_RESULT = bytes.fromhex("0c 0200000a 1e 29 4d 4e 73 00 61 62 4f 29 4c 39 00 5e 91 02 91 20 5f 1f")


class MixedWorkloadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.loop = asyncio.get_running_loop()
        self.peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(self.peer.close)
        self.peer.bind(("127.0.0.1", 0))
        self.peer.setblocking(False)
        self.router = "%s:%d" % self.peer.getsockname()
        self.target = rb.RoutedTarget(NETWORK, STATION, router=self.router)
        self.endpoint = rb.BipEndpoint(
            device_instance=100, interface="127.0.0.1", port=0,
            broadcast_address="127.0.0.1", segmentation=rb.Segmentation.SEGMENTED_RECEIVE,
            apdu_retries=0, apdu_timeout_ms=30000,
        )
        await self.endpoint.start()
        self.addAsyncCleanup(self.close_endpoint)
        ip, port = (await self.endpoint.local_address()).rsplit(":", 1)
        self.socket_address = (ip, int(port))
        self.client = await self.endpoint.client(confirmed_cov_subscription_limit=1)

    async def close_endpoint(self):
        await self.endpoint.close()

    async def routed_frame(self):
        wire, source = await sock_recvfrom(self.loop, self.peer, 2048)
        self.assertEqual(source, self.socket_address)
        return request_of(wire)

    async def receive(self, station=STATION, network=NETWORK):
        frame = await self.routed_frame()
        self.assertEqual((frame["dnet"], frame["dadr"]), (network, station))
        return frame["apdu"]

    async def send(self, apdu, station=STATION, network=NETWORK):
        await sock_sendto(self.loop, self.peer, relay(network, station, apdu), self.socket_address)

    async def cov_segment(self, invoke, seq, body):
        more = seq == 0
        data = body[:8] if more else body[8:]
        await self.send(bytes((0x0C if more else 0x08, 5, invoke, seq, 2, 1)) + data)
        self.assertEqual(await self.receive(), bytes((0x41, invoke, seq, 2)))

    async def assert_event(self, events, value):
        event = await anext(events)
        self.assertEqual(event.subscriber_process_identifier, PROCESS)
        self.assertEqual(event.initiating_device_identifier, rb.ObjectIdentifier(rb.ObjectType.DEVICE, FAR))
        self.assertEqual(event.monitored_object_identifier, AI_1)
        self.assertEqual((event.source_network, event.source_address, event.source_mac),
                         (NETWORK, STATION, mac(self.router)))
        self.assertEqual([item["value"] for item in event.values], [rb.PropertyValue.real(value)])

    def read_multiple(self, target):
        return self.client.read_property_multiple(
            target, [(DEVICE, [(P.OBJECT_NAME, None), (P.OBJECT_LIST, 0)])])

    def read_page(self, target):
        return self.client.read_range(
            target, TREND_LOG, P.LOG_BUFFER, range_type="sequence",
            reference_seq=1, count=5, validation="lenient")

    async def cancel_read(self, read):
        # The binding installs its abort callback before returning this Future.
        # Let it request native cancellation before the peer continues.
        callbacks_run = asyncio.Event()
        read.add_done_callback(lambda _: callbacks_run.set())
        read.cancel()
        await callbacks_run.wait()
        with self.assertRaises(asyncio.CancelledError):
            await read

    async def register_cov(self):
        events = await self.client.cov_notifications()
        lease = await self.client.expect_confirmed_cov(
            self.target, PROCESS, AI_1, initiating_device_instance=FAR)
        self.addCleanup(lease.close)
        return events

    async def test_routed_reads_and_confirmed_cov_share_a_socket_under_load(self):
        # Every transition waits on a datagram or a completed operation. The outer
        # deadline just bounds a broken peer or missing wakeup in the test suite.
        async with async_timeout(60):
            events = await self.register_cov()
            frames = await self.endpoint.frames(capacity=1)
            data = bytes(range(256)) * 4
            # A routed path admits one outbound request at a time. Four DNETs
            # behind the same router keep all four requests live on this socket.
            targets = {service: rb.RoutedTarget(network, STATION, router=self.router)
                       for service, network in READ_NETWORK.items()}
            rp = self.client.read_property(targets[RP], AI_1, P.PRESENT_VALUE)
            rpm = self.read_multiple(targets[RPM])
            page = self.read_page(targets[READ_RANGE])
            file_read = self.client.atomic_read_file(
                targets[READ_FILE], FILE_1, "stream", start_position=40, requested_octet_count=len(data))

            requests = {}
            for _ in range(4):
                frame = await self.routed_frame()
                apdu = frame["apdu"]
                self.assertEqual(apdu[0] >> 4, 0)
                self.assertNotIn(apdu[3], requests)
                self.assertEqual((frame["dnet"], frame["dadr"]), (READ_NETWORK[apdu[3]], STATION))
                requests[apdu[3]] = apdu
            self.assertEqual(set(requests), {RP, RPM, READ_RANGE, READ_FILE})
            self.assertEqual((await self.endpoint.status())["active_leases"], 4)
            self.assertEqual(parse_read(requests[READ_FILE][4:]), ("stream", 40, len(data)))

            await self.cancel_read(rp)
            # Even a valid late answer for the cancelled call can't satisfy a sibling.
            await self.send(bytes((0x30, requests[RP][2], RP)) + requests[RP][4:]
                            + bytes.fromhex("3e 44 3f800000 3f"), network=READ_NETWORK[RP])

            # The station is simultaneously the file responder and COV requester.
            invoke = requests[READ_FILE][2]
            pieces = segments(invoke, stream_ack(40, data, True), 300, 2)
            self.assertEqual(len(pieces), 4)
            await self.send(pieces[0])
            self.assertEqual(await self.receive(), bytes((0x40, invoke, 0, 2)))
            body = notification(PROCESS, 21.5)[2:]
            await self.cov_segment(invoke, 0, body)

            await self.send(request_of(wrapped_page_ack(requests[READ_RANGE][2]))["apdu"],
                            network=READ_NETWORK[READ_RANGE])
            result = await page
            self.assertEqual((result["item_count"], result["first_sequence_number"], result["item_data"]),
                             (1, 0, TREND_RECORD))
            self.assertEqual(list(result["result_flags"].bits), [True, True, False])
            self.assertEqual(result["violations"], ["zero_first_sequence_number"])

            # Withhold segment one. The peer retransmits the window on the NAK.
            await self.send(pieces[2])
            self.assertEqual(await self.receive(), bytes((0x42, invoke, 0, 2)))
            await self.send(pieces[1])
            await self.send(pieces[2])
            self.assertEqual(await self.receive(), bytes((0x40, invoke, 2, 2)))
            await self.cov_segment(invoke, 1, body)
            self.assertEqual(await self.receive(), bytes((0x20, invoke, 1)))
            await self.assert_event(events, 21.5)

            self.assertEqual(requests[RPM][4:], RPM_REQUEST)
            await self.send(bytes((0x30, requests[RPM][2], RPM)) + RPM_RESULT, network=READ_NETWORK[RPM])
            [item] = await rpm
            self.assertEqual(item["object_id"], DEVICE)
            first, second = item["results"]
            self.assertEqual(first, dict(property_id=P.OBJECT_NAME, array_index=None,
                                         value=rb.PropertyValue.character_string("ab"), error=None, decode_error=None))
            self.assertEqual((second["property_id"], second["array_index"], second["value"]),
                             (P.OBJECT_LIST, 0, None))
            self.assertEqual(tuple(map(int, second["error"])), (2, 32))

            await self.send(pieces[3])
            self.assertEqual(await self.receive(), bytes((0x40, invoke, 3, 2)))
            self.assertEqual(await file_read, dict(access_method="stream", start_position=40,
                                                   data=data, end_of_file=True))
            # Otherwise identical COV from another station can't use the lease.
            other = b"\x0b"
            await self.send(bytes((0, 5, 81, 1)) + notification(PROCESS, 99.0)[2:], other)
            self.assertEqual(await self.receive(other), bytes.fromhex("50 51 01 91 05 91 4f"))
            await self.send(bytes((0, 5, 82, 1)) + notification(PROCESS, 22.5)[2:])
            self.assertEqual(await self.receive(), bytes.fromhex("20 52 01"))
            await self.assert_event(events, 22.5)
            self.assertEqual(events.missed, 0)
            self.assertGreater(frames.missed, 0)

            followup = self.client.read_property(self.target, AI_1, P.PRESENT_VALUE)
            request = await self.receive()
            self.assertEqual((request[0] >> 4, request[3]), (0, RP))
            await self.send(bytes((0x30, request[2], RP)) + request[4:] + bytes.fromhex("3e 44 42000000 3f"))
            self.assertEqual(await followup, rb.PropertyValue.real(32.0))
            status = await self.endpoint.status()
            self.assertEqual(status["active_leases"], 0)
            self.assertTrue(status["transport_health"]["ready"])

            # Stop during a new confirmed call. Attach an empty frame subscription
            # after its outbound request so both pending iterator reads need a wakeup.
            withheld = self.client.read_property(self.target, AI_1, P.PRESENT_VALUE)
            request = await self.receive()
            self.assertEqual((request[0] >> 4, request[3]), (0, RP))
            waiting_frames = await self.endpoint.frames(capacity=1)
            next_frame = anext(waiting_frames)
            next_event = anext(events)
            await self.endpoint.close()
            with self.assertRaisesRegex(rb.BacnetError, "the client stopped while it waited"):
                await withheld
            for pending in (next_frame, next_event, anext(frames)):
                with self.assertRaises(StopAsyncIteration):
                    await pending

    async def test_one_router_path_serializes_reads_while_serving_segmented_cov(self):
        async with async_timeout(60):
            events = await self.register_cov()
            file_read = self.client.atomic_read_file(
                self.target, FILE_1, "stream", start_position=40, requested_octet_count=4)
            file_request = await self.receive()
            self.assertEqual((file_request[0] >> 4, file_request[3]), (0, READ_FILE))
            invoke = file_request[2]
            # All three target the same (router, DNET) as the file transfer.
            rp = self.client.read_property(self.target, AI_1, P.PRESENT_VALUE)
            rpm = self.read_multiple(self.target)
            page = self.read_page(self.target)
            body = notification(PROCESS, 19.0)[2:]
            await self.cov_segment(invoke, 0, body)
            self.assertEqual((await self.endpoint.status())["active_leases"], 1)
            self.assertTrue(all(not pending.done() for pending in (rp, rpm, page, file_read)))
            await self.cancel_read(rp)
            await self.cov_segment(invoke, 1, body)
            self.assertEqual(await self.receive(), bytes((0x20, invoke, 1)))
            await self.assert_event(events, 19.0)

            await self.send(bytes((0x30, invoke, READ_FILE)) + stream_ack(40, b"done", True))
            self.assertEqual((await file_read)["data"], b"done")
            expected = {RPM, READ_RANGE}
            for _ in range(2):
                request = await self.receive()
                self.assertEqual(request[0] >> 4, 0)
                service = request[3]
                self.assertIn(service, expected)
                expected.remove(service)
                self.assertEqual((await self.endpoint.status())["active_leases"], 1)
                if service == RPM:
                    self.assertEqual(request[4:], RPM_REQUEST)
                    await self.send(bytes((0x30, request[2], RPM)) + RPM_RESULT)
                    self.assertEqual((await rpm)[0]["object_id"], DEVICE)
                else:
                    await self.send(request_of(wrapped_page_ack(request[2]))["apdu"])
                    self.assertEqual((await page)["item_data"], TREND_RECORD)

            # End the observed path trace with a distinct reply. Any intervening
            # RP, duplicate service request or fallback fails this assertion.
            await self.send(bytes((0, 5, 82, 1)) + notification(PROCESS, 20.0)[2:])
            self.assertEqual(await self.receive(), bytes.fromhex("20 52 01"))
            await self.assert_event(events, 20.0)
            status = await self.endpoint.status()
            self.assertEqual(status["active_leases"], 0)
            self.assertTrue(status["transport_health"]["ready"])
            await self.endpoint.close()
            with self.assertRaises(StopAsyncIteration):
                await anext(events)
