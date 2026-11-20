"""Capture the running node's plaintext and independently compare the mTLS peer's bytes."""
import asyncio
import gc
import math
import socket
import time

from rusty_bacnet import AceMixedRouter, RouterBipPort, RouterScPort, ScEndpoint, ScReconnect
from sc_client_fixture import ClientScFixture, mtls
import test_frames

NODE = bytes.fromhex("020000000002")
REMOTE = bytes.fromhex("020000000004")


async def next_matching(frames, predicate):
    async with asyncio.timeout(30):
        async for frame in frames:
            if predicate(frame):
                return frame
    raise AssertionError("capture ended before the expected frame")


class ScFrameTests(ClientScFixture):
    async def close_owner(self, close):
        await close()

    async def hub_peer(self):
        connections = asyncio.Queue()

        async def handle(reader, writer, connect):
            await self.accept_sc(writer, connect)
            connections.put_nowait((reader, writer, connect))
            await asyncio.Event().wait()

        return await self.raw_peer(handle), connections

    def router(self, url, **options):
        sc = RouterScPort(200, url, NODE, mtls.CLIENT_UUID, self.path("site.pem"),
                          self.path("client.pem"), self.path("client.key"), **options)
        bip = RouterBipPort(100, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1")
        return AceMixedRouter.from_ports([bip, sc], isolate_sc_ports=True)

    def endpoint(self, url, **options):
        return ScEndpoint(9001, sc_hub=url, sc_vmac=NODE, sc_device_uuid=mtls.CLIENT_UUID,
                          sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("client.pem"),
                          sc_client_key=self.path("client.key"), **options)

    async def peer_message(self, reader, predicate):
        async with asyncio.timeout(30):
            while True:
                opcode, payload = await self.frame(reader, True)
                self.assertEqual(opcode, 2)
                if predicate(payload):
                    return payload

    def check_sc(self, frame, direction, url, index, role="primary"):
        self.assertEqual((frame.direction, frame.link), (direction, "sc"))
        self.assertEqual((frame.port_index, frame.local_address), (index, NODE.hex(":")))
        self.assertEqual((frame.peer, frame.hub_url), (role, url))
        self.assertIsInstance(frame.timestamp_unix_ns, int)

    async def test_mixed_router_taps_both_sides_of_forwarding_and_link_controls(self):
        url, connections = await self.hub_peer()
        router = self.router(url)
        self.addAsyncCleanup(self.close_owner, router.stop)
        with self.assertRaises(RuntimeError):
            await router.frames(1)
        await router.start()
        reader, writer, _ = await connections.get()
        with self.assertRaises(IndexError):
            await router.frames(2)
        with self.assertRaises(ValueError):
            router.frames(1, capacity=0)
        bip, sc, slow = await router.frames(0), await router.frames(1), await router.frames(1, 1)
        loop = asyncio.get_running_loop()
        udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(udp.close)
        udp.bind(("127.0.0.1", 0))
        udp.setblocking(False)
        ip, port = (await router.local_address()).split(":")
        # Address a Who-Is on B/IP to a station on the attached SC network.
        who_is = b"\x10\x08"
        npdu = b"\x01\x20\x00\xc8\x06" + REMOTE + b"\x10" + who_is
        datagram = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        await loop.sock_sendto(udp, datagram, (ip, int(port)))
        wire = await self.peer_message(reader, lambda p: p.startswith(b"\x01")
                                       and p.endswith(who_is))
        forwarded = await next_matching(sc, lambda f: f.octets == wire)
        self.check_sc(forwarded, "outbound", url, 1)
        ingress = await next_matching(bip, lambda f: f.octets == datagram)
        self.assertEqual((ingress.direction, ingress.port_index, ingress.hub_url),
                         ("inbound", 0, None))
        self.assertEqual(ingress.local_address, await router.local_address())
        self.assertEqual(ingress.peer, "%s:%d" % udp.getsockname())

        # A Who-Is-Router-To-Network for network 100 produces an I-Am-Router reply.
        query = b"\x01\x08\x13\x57" + REMOTE + b"\x01\x80\x00\x00\x64"
        await self.send_frame(writer, query)
        response = await self.peer_message(reader, lambda p: p.endswith(b"\x01\x80\x01\x00\x64"))
        self.check_sc(await next_matching(sc, lambda f: f.octets == query), "inbound", url, 1)
        self.check_sc(await next_matching(sc, lambda f: f.octets == response), "outbound", url, 1)
        self.assertGreater(slow.missed, 0)
        await slow.aclose()

        # A BVLC-SC control is exchanged on the same connection after detach.
        heartbeat, ack = b"\x0a\0\x24\x68", b"\x0b\0\x24\x68"
        before = time.monotonic(), time.time()
        await self.send_frame(writer, heartbeat)
        await self.peer_message(reader, lambda p: p == ack)
        after = time.monotonic(), time.time()
        controls = []
        for octets, direction in ((heartbeat, "inbound"), (ack, "outbound")):
            frame = await next_matching(sc, lambda f: f.octets == octets)
            self.check_sc(frame, direction, url, 1)
            controls.append(frame)
            self.assertAlmostEqual(frame.timestamp_unix_ns / 1_000_000_000, frame.timestamp,
                                   delta=math.ulp(frame.timestamp))
        # The peer saw the ACK before Python consumed either captured record.
        test_frames.FrameTests.assert_captured_between(self, controls, before, after)

        pending = anext(sc)
        await router.stop()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 30)
        await router.start()
        fresh = await router.frames(1)
        reader, writer, _ = await connections.get()
        await self.send_frame(writer, heartbeat)
        await self.peer_message(reader, lambda p: p == ack)
        self.check_sc(await next_matching(fresh, lambda f: f.octets == ack), "outbound", url, 1)
        with self.assertRaises(StopAsyncIteration):
            await anext(sc)
        self.assertTrue(connections.empty())

    async def test_endpoint_malformed_frame_and_exact_trailing_overflow(self):
        url, connections = await self.hub_peer()
        endpoint = self.endpoint(url)
        self.addAsyncCleanup(self.close_owner, endpoint.close)
        with self.assertRaises(RuntimeError):
            await endpoint.frames()
        await endpoint.start()
        reader, writer, _ = await connections.get()
        frames, slow = await endpoint.frames(), await endpoint.frames(1)
        pending = anext(frames)
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        # A truncated header reaches the observer before BVLC-SC decoding.
        client = await endpoint.client()
        client_frames = await client.frames()
        malformed = b"\x01"
        await self.send_frame(writer, malformed)
        self.check_sc(await next_matching(frames, lambda f: f.octets == malformed),
                      "inbound", url, None)
        self.check_sc(await next_matching(client_frames, lambda f: f.octets == malformed),
                      "inbound", url, None)
        await client_frames.aclose()
        # The witness and the peer fence each heartbeat without polling a clock.
        for n in range(4):
            request, ack = bytes([10, 0, 0, n]), bytes([11, 0, 0, n])
            await self.send_frame(writer, request)
            await self.peer_message(reader, lambda p: p == ack)
            for payload in (request, ack):
                self.assertEqual((await asyncio.wait_for(anext(frames), 30)).octets, payload)
        self.assertEqual(slow.missed, 8)
        # Read no retained frame. The last lost record is counted even after detach.
        await slow.aclose()
        self.assertEqual(slow.missed, 8)
        self.assertEqual(frames.missed, 0)
        pending = anext(frames)
        await endpoint.close()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 30)
        await endpoint.start()
        fresh = await endpoint.frames()
        _, writer, _ = await connections.get()
        await self.send_frame(writer, malformed)
        self.check_sc(await next_matching(fresh, lambda f: f.octets == malformed),
                      "inbound", url, None)
        with self.assertRaises(StopAsyncIteration):
            await anext(frames)

    async def test_queued_primary_frames_keep_their_url_after_failover(self):
        for owner_kind in ("router", "endpoint"):
            with self.subTest(owner=owner_kind):
                primary, primary_connections = await self.hub_peer()
                relay = await self.relay(primary)
                failover, failover_connections = await self.hub_peer()
                if owner_kind == "router":
                    index = 1
                    owner = self.router(relay.url, failover_hub=failover,
                                        reconnect_initial_delay_ms=50, reconnect_max_delay_ms=50,
                                        reconnect_max_retries=2**32 - 1, reconnect_forever=True)
                    close = owner.stop
                else:
                    index = None
                    owner = self.endpoint(relay.url, sc_failover_hub=failover,
                                          reconnect=ScReconnect(initial_delay_ms=50,
                                                                max_delay_ms=50, max_retries=1))
                    close = owner.close
                self.addAsyncCleanup(self.close_owner, close)
                await owner.start()
                frames = await (owner.frames(index) if index is not None else owner.frames())
                reader, writer, _ = await primary_connections.get()
                primary_request = bytes.fromhex("0a00a123")
                primary_ack = bytes.fromhex("0b00a123")
                await self.send_frame(writer, primary_request)
                await self.peer_message(reader, lambda p: p == primary_ack)
                await relay.close()

                # Wait for the second socket to exchange traffic before reading
                # any capture record, including those queued on the first.
                reader, writer, connect = await asyncio.wait_for(failover_connections.get(), 30)
                failover_request = bytes.fromhex("0a00b123")
                failover_ack = bytes.fromhex("0b00b123")
                await self.send_frame(writer, failover_request)
                await self.peer_message(reader, lambda p: p == failover_ack)
                for octets, direction, hub, role in (
                    (primary_request, "inbound", relay.url, "primary"),
                    (primary_ack, "outbound", relay.url, "primary"),
                    (connect, "outbound", failover, "failover"),
                    (failover_request, "inbound", failover, "failover"),
                    (failover_ack, "outbound", failover, "failover"),
                ):
                    frame = await next_matching(frames, lambda f: f.octets == octets)
                    self.check_sc(frame, direction, hub, index, role)
                self.assertEqual(frames.missed, 0)
                await close()

    async def test_dropping_router_or_endpoint_wakes_observer(self):
        url, connections = await self.hub_peer()
        for build, args in ((self.router, (1,)), (self.endpoint, ())):
            owner = build(url)
            await owner.start()
            reader, writer, _ = await connections.get()
            frames = await owner.frames(*args)
            # Drain observations through an in-band marker before testing a pending read.
            await self.send_frame(writer, b"\x0a\0\x12\x34")
            await self.peer_message(reader, lambda p: p == b"\x0b\0\x12\x34")
            await next_matching(frames, lambda f: f.octets == b"\x0b\0\x12\x34")
            pending = anext(frames)
            del owner
            gc.collect()
            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(pending, 30)
