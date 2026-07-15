"""Installed-wheel BACnet/IP loopback smoke tests."""

from __future__ import annotations

import asyncio
import socket
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetAbortError,
    BacnetError,
    BacnetNotificationLagError,
    BacnetProtocolError,
    BacnetRejectError,
    BacnetTimeoutError,
    DirectTarget,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    RoutedTarget,
)


class _FaultResponder(asyncio.DatagramProtocol):
    """Return one correlated Reject or Abort for a direct confirmed request."""

    def __init__(self, response_kind: str) -> None:
        self.response_kind = response_kind
        self.received = asyncio.Event()

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) < 9 or data[:2] != b"\x81\x0a":
            return
        invoke_id = data[8]
        if self.response_kind == "reject":
            apdu = bytes((0x60, invoke_id, 0x04))  # invalid-tag
        elif self.response_kind == "abort":
            apdu = bytes((0x71, invoke_id, 0x01))  # server, buffer-overflow
        else:
            apdu = bytes((0xF0, invoke_id))  # reserved/invalid APDU type
        npdu = b"\x01\x00" + apdu
        frame = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        self.transport.sendto(frame, address)
        self.received.set()

    def connection_made(self, transport) -> None:
        self.transport = transport


class _ManagedCovFailureResponder(asyncio.DatagramProtocol):
    """ACK initial/cancel SubscribeCOV but drop the renewal request."""

    def __init__(self) -> None:
        self.request_count = 0
        self.initial_request_length = None
        self.cancellation_count = 0

    def connection_made(self, transport) -> None:
        self.transport = transport

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) < 10 or data[:2] != b"\x81\x0a":
            return
        npdu = data[4:]
        if len(npdu) < 6 or npdu[0] != 0x01 or npdu[2] >> 4 != 0:
            return
        invoke_id = npdu[4]
        service = npdu[5]
        if service != 5:  # SubscribeCOV
            return
        self.request_count += 1
        service_request_length = len(npdu[6:])
        if self.initial_request_length is None:
            self.initial_request_length = service_request_length
        elif service_request_length >= self.initial_request_length:
            # Drop the renewal and every TSM retry. A cancellation omits the
            # confirmed-notification and lifetime fields, so it is shorter.
            return
        else:
            self.cancellation_count += 1
        apdu = bytes((0x20, invoke_id, service))
        response_npdu = b"\x01\x00" + apdu
        frame = b"\x81\x0a" + (len(response_npdu) + 4).to_bytes(2, "big")
        self.transport.sendto(frame + response_npdu, address)


class _RawRpmResponder(asyncio.DatagramProtocol):
    """Return an RPM value containing an unknown application tag verbatim."""

    def connection_made(self, transport) -> None:
        self.transport = transport

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) < 10 or data[:2] != b"\x81\x0a":
            return
        npdu = data[4:]
        if len(npdu) < 6 or npdu[0] != 1 or npdu[2] >> 4 != 0 or npdu[5] != 14:
            return
        invoke_id = npdu[4]
        rpm_ack = (
            b"\x0c\x00\x00\x00\x01"  # context 0: analog-input,1
            b"\x1e"  # opening context 1: list of results
            b"\x29\x55"  # context 2: present-value (85)
            b"\x4e\xd1\xab\x4f"  # context 4: unknown app tag 13 + byte
            b"\x1f"  # closing context 1
        )
        apdu = bytes((0x30, invoke_id, 14)) + rpm_ack
        response_npdu = b"\x01\x00" + apdu
        frame = b"\x81\x0a" + (len(response_npdu) + 4).to_bytes(2, "big")
        self.transport.sendto(frame + response_npdu, address)


class _RouterProxy(asyncio.DatagramProtocol):
    """Forward one routed network as direct B/IP and restore routed responses."""

    def __init__(self, server_address, network: int, dadr: bytes) -> None:
        self.server_address = server_address
        self.network = network
        self.dadr = dadr
        self.client_address = None
        self.routed_requests = []

    def connection_made(self, transport) -> None:
        self.transport = transport

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) < 6 or data[:2] != b"\x81\x0a":
            return
        npdu = data[4:]
        control = npdu[1]
        if control & 0x20:
            offset = 2
            network = int.from_bytes(npdu[offset : offset + 2], "big")
            offset += 2
            dlen = npdu[offset]
            offset += 1
            dadr = npdu[offset : offset + dlen]
            offset += dlen + 1  # DADR and hop count
            self.client_address = address
            self.routed_requests.append((network, dadr, npdu[offset:]))
            direct_npdu = b"\x01\x04" + npdu[offset:]
            frame = b"\x81\x0a" + (len(direct_npdu) + 4).to_bytes(2, "big")
            self.transport.sendto(frame + direct_npdu, self.server_address)
            return

        if address == self.server_address and self.client_address is not None:
            apdu = npdu[2:]
            routed_npdu = (
                b"\x01\x08"
                + self.network.to_bytes(2, "big")
                + bytes((len(self.dadr),))
                + self.dadr
                + apdu
            )
            frame = b"\x81\x0a" + (len(routed_npdu) + 4).to_bytes(2, "big")
            self.transport.sendto(frame + routed_npdu, self.client_address)


class BipLoopbackTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = BACnetServer(
            device_instance=4_194_001,
            device_name="Installed Wheel Loopback",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        self.server.add_analog_input(
            instance=1,
            name="Loopback Temperature",
            units=62,
            present_value=21.5,
        )
        self.server.add_analog_output(
            instance=1,
            name="Loopback Setpoint",
            units=62,
        )
        self.server.add_character_string_value(
            instance=1,
            name="Loopback Long Text",
        )
        await self.server.start()
        self.address = await self.server.local_address()

    async def asyncTearDown(self) -> None:
        await self.server.stop()

    async def test_direct_rp_rpm_wp_and_discovery_request(self) -> None:
        analog_input = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            present_value = await client.read_property(
                self.address,
                analog_input,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertEqual(present_value.tag, "real")
            self.assertAlmostEqual(present_value.value, 21.5)

            await client.write_property(
                self.address,
                analog_output,
                PropertyIdentifier.PRESENT_VALUE,
                PropertyValue.real(18.25),
                priority=8,
            )
            written_value = await client.read_property(
                self.address,
                analog_output,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertAlmostEqual(written_value.value, 18.25)

            rpm_results = await client.read_property_multiple(
                self.address,
                [
                    (
                        analog_input,
                        [
                            (PropertyIdentifier.OBJECT_NAME, None),
                            (PropertyIdentifier.PRESENT_VALUE, None),
                        ],
                    ),
                    (
                        analog_output,
                        [(PropertyIdentifier.PRESENT_VALUE, None)],
                    ),
                ],
            )
            self.assertEqual(len(rpm_results), 2)
            self.assertEqual(
                rpm_results[0]["results"][0]["value"].value,
                "Loopback Temperature",
            )
            self.assertAlmostEqual(
                rpm_results[0]["results"][1]["value"].value,
                21.5,
            )
            self.assertAlmostEqual(
                rpm_results[1]["results"][0]["value"].value,
                18.25,
            )

            # With both endpoints using OS-assigned ports, the server's I-Am
            # broadcast cannot reliably return to this client on one host.
            # Still exercise the installed discovery request/collection path;
            # response assertions belong in the later network-topology fixture.
            devices = await client.discover(timeout_ms=50)
            self.assertIsInstance(devices, list)

    async def test_typed_direct_and_routed_rp_rpm_wp_wpm(self) -> None:
        analog_input = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        server_host, server_port = self.address.rsplit(":", 1)
        remote_network = 2001
        remote_dadr = b"\x00\x7f\x01"
        proxy = _RouterProxy(
            (server_host, int(server_port)), remote_network, remote_dadr
        )
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: proxy,
            local_addr=("127.0.0.1", 0),
        )
        router_address = "{}:{}".format(*transport.get_extra_info("sockname"))

        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            ) as client:
                direct = DirectTarget(self.address)
                direct_value = await client.read_property(
                    direct, analog_input, PropertyIdentifier.PRESENT_VALUE
                )
                self.assertAlmostEqual(direct_value.value, 21.5)

                routed = RoutedTarget(router_address, remote_network, remote_dadr)
                routed_value = await client.read_property(
                    routed, analog_input, PropertyIdentifier.PRESENT_VALUE
                )
                self.assertAlmostEqual(routed_value.value, 21.5)

                rpm_results = await client.read_property_multiple(
                    routed,
                    [
                        (
                            analog_input,
                            [(PropertyIdentifier.PRESENT_VALUE, None)],
                        )
                    ],
                )
                self.assertAlmostEqual(
                    rpm_results[0]["results"][0]["value"].value, 21.5
                )

                await client.write_property(
                    routed,
                    analog_output,
                    PropertyIdentifier.PRESENT_VALUE,
                    PropertyValue.real(19.5),
                    priority=8,
                )
                await client.write_property_multiple(
                    routed,
                    [
                        (
                            analog_output,
                            [
                                (
                                    PropertyIdentifier.PRESENT_VALUE,
                                    PropertyValue.real(20.25),
                                    8,
                                    None,
                                )
                            ],
                        )
                    ],
                )
                written = await client.read_property(
                    routed, analog_output, PropertyIdentifier.PRESENT_VALUE
                )
                self.assertAlmostEqual(written.value, 20.25)

            self.assertGreaterEqual(len(proxy.routed_requests), 5)
            for network, dadr, _apdu in proxy.routed_requests:
                self.assertEqual(network, remote_network)
                self.assertEqual(dadr, remote_dadr)
        finally:
            transport.close()

    async def test_rpm_preserves_per_property_errors_order_and_array_indexes(self) -> None:
        analog_input = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        proprietary_property = PropertyIdentifier.from_raw(600)
        requested = [
            (PropertyIdentifier.PRESENT_VALUE, None),
            (proprietary_property, 7),
            (PropertyIdentifier.OBJECT_NAME, None),
        ]

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            response = await client.read_property_multiple(
                self.address, [(analog_input, requested)]
            )

        results = response[0]["results"]
        self.assertEqual(
            [(item["property_id"].to_raw(), item["array_index"]) for item in results],
            [(85, None), (600, 7), (77, None)],
        )
        self.assertIsNotNone(results[0]["value"])
        self.assertIsNone(results[0]["error"])
        self.assertIsNone(results[1]["value"])
        self.assertIsNotNone(results[1]["error"])
        self.assertEqual(results[1]["error"][0].to_raw(), 2)  # property
        self.assertEqual(results[1]["error"][1].to_raw(), 32)  # unknown-property
        self.assertEqual(results[2]["value"].value, "Loopback Temperature")

    async def test_rpm_preserves_unknown_raw_value_bytes(self) -> None:
        responder = _RawRpmResponder()
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: responder,
            local_addr=("127.0.0.1", 0),
        )
        address = "{}:{}".format(*transport.get_extra_info("sockname"))
        analog_input = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            ) as client:
                response = await client.read_property_multiple(
                    address,
                    [(analog_input, [(PropertyIdentifier.PRESENT_VALUE, None)])],
                )
            result = response[0]["results"][0]
            self.assertEqual(result["property_id"], PropertyIdentifier.PRESENT_VALUE)
            self.assertEqual(result["value"], b"\xd1\xab")
            self.assertIsNone(result["error"])
        finally:
            transport.close()

    async def test_typed_direct_and_routed_segmented_responses(self) -> None:
        string_value = ObjectIdentifier(ObjectType.CHARACTERSTRING_VALUE, 1)
        long_value = "segmented-response-" * 180

        server_host, server_port = self.address.rsplit(":", 1)
        remote_network = 2002
        remote_dadr = b"\x01\x02\x03"
        proxy = _RouterProxy(
            (server_host, int(server_port)), remote_network, remote_dadr
        )
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: proxy,
            local_addr=("127.0.0.1", 0),
        )
        router_address = "{}:{}".format(*transport.get_extra_info("sockname"))

        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            ) as client:
                direct = DirectTarget(self.address)
                await client.write_property(
                    direct,
                    string_value,
                    PropertyIdentifier.PRESENT_VALUE,
                    PropertyValue.character_string(long_value),
                )
                direct_result = await client.read_property(
                    direct, string_value, PropertyIdentifier.PRESENT_VALUE
                )
                self.assertEqual(direct_result.value, long_value)

                routed = RoutedTarget(router_address, remote_network, remote_dadr)
                routed_result = await client.read_property(
                    routed, string_value, PropertyIdentifier.PRESENT_VALUE
                )
                self.assertEqual(routed_result.value, long_value)

            self.assertGreaterEqual(len(proxy.routed_requests), 2)
            self.assertTrue(
                any((apdu[0] & 0xF0) == 0x40 for _, _, apdu in proxy.routed_requests),
                "routed segmented response must produce a routed SegmentACK",
            )
        finally:
            transport.close()

    async def test_confirmed_cov_notification(self) -> None:
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            notifications = await client.cov_notifications()
            await client.subscribe_cov(
                self.address,
                subscriber_process_identifier=41,
                monitored_object_identifier=analog_output,
                confirmed=True,
                lifetime=60,
            )

            await client.write_property(
                self.address,
                analog_output,
                PropertyIdentifier.PRESENT_VALUE,
                PropertyValue.real(22.75),
                priority=8,
            )

            async def wait_for_changed_value():
                async for notification in notifications:
                    if notification.monitored_object_identifier != analog_output:
                        continue
                    for value in notification.values:
                        if value["property_id"] != PropertyIdentifier.PRESENT_VALUE:
                            continue
                        property_value = value["value"]
                        if property_value is not None and property_value.value == 22.75:
                            return notification

            notification = await asyncio.wait_for(wait_for_changed_value(), timeout=3)
            self.assertEqual(notification.delivery, "confirmed")
            self.assertEqual(notification.subscriber_process_identifier, 41)

            await client.unsubscribe_cov(
                self.address,
                subscriber_process_identifier=41,
                monitored_object_identifier=analog_output,
            )

    async def test_unconfirmed_cov_initial_notification_and_observable_lag(self) -> None:
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            notifications = await client.cov_notifications()
            await client.subscribe_cov(
                self.address,
                subscriber_process_identifier=45,
                monitored_object_identifier=analog_output,
                confirmed=False,
                lifetime=60,
            )

            initial = await asyncio.wait_for(notifications.__anext__(), timeout=3)
            self.assertEqual(initial.delivery, "unconfirmed")
            self.assertEqual(initial.subscriber_process_identifier, 45)
            self.assertEqual(initial.monitored_object_identifier, analog_output)
            self.assertTrue(
                any(
                    value["property_id"] == PropertyIdentifier.PRESENT_VALUE
                    for value in initial.values
                )
            )

            for value in range(70):
                await client.write_property(
                    self.address,
                    analog_output,
                    PropertyIdentifier.PRESENT_VALUE,
                    PropertyValue.real(float(value)),
                    priority=8,
                )

            with self.assertRaises(BacnetNotificationLagError) as raised:
                await asyncio.wait_for(notifications.__anext__(), timeout=3)
            self.assertGreater(raised.exception.skipped, 0)

            await client.unsubscribe_cov(
                self.address,
                subscriber_process_identifier=45,
                monitored_object_identifier=analog_output,
            )

    async def test_routed_confirmed_cov_without_discovery_priming(self) -> None:
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        server_host, server_port = self.address.rsplit(":", 1)
        remote_network = 2002
        remote_dadr = b"\x00\x44\x02"
        proxy = _RouterProxy(
            (server_host, int(server_port)), remote_network, remote_dadr
        )
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: proxy,
            local_addr=("127.0.0.1", 0),
        )
        router_address = "{}:{}".format(*transport.get_extra_info("sockname"))
        routed = RoutedTarget(router_address, remote_network, remote_dadr)

        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            ) as client:
                notifications = await client.cov_notifications()
                await client.subscribe_cov(
                    routed,
                    subscriber_process_identifier=42,
                    monitored_object_identifier=analog_output,
                    confirmed=True,
                    lifetime=60,
                )
                await client.write_property(
                    routed,
                    analog_output,
                    PropertyIdentifier.PRESENT_VALUE,
                    PropertyValue.real(23.25),
                    priority=8,
                )

                async def wait_for_changed_value():
                    async for notification in notifications:
                        if notification.monitored_object_identifier != analog_output:
                            continue
                        for value in notification.values:
                            property_value = value["value"]
                            if property_value is not None and property_value.value == 23.25:
                                return notification

                notification = await asyncio.wait_for(wait_for_changed_value(), timeout=3)
                self.assertEqual(notification.delivery, "confirmed")
                self.assertEqual(notification.source_network, remote_network)
                self.assertEqual(notification.source_address, remote_dadr)

                await client.unsubscribe_cov(
                    routed,
                    subscriber_process_identifier=42,
                    monitored_object_identifier=analog_output,
                )
        finally:
            transport.close()

    async def test_managed_routed_cov_renews_and_closes_without_discovery(self) -> None:
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        server_host, server_port = self.address.rsplit(":", 1)
        remote_network = 2003
        remote_dadr = b"\x00\x45\x03"
        proxy = _RouterProxy(
            (server_host, int(server_port)), remote_network, remote_dadr
        )
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: proxy,
            local_addr=("127.0.0.1", 0),
        )
        router_address = "{}:{}".format(*transport.get_extra_info("sockname"))

        try:
            client = BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            )
            async with client:
                pass

            async with client:
                handle = await client.manage_cov_subscription(
                    RoutedTarget(router_address, remote_network, remote_dadr),
                    subscriber_process_identifier=43,
                    monitored_object_identifier=analog_output,
                    confirmed=True,
                    lifetime=2,
                    renewal_margin_ms=1000,
                )
                self.assertFalse(handle.closed)
                events = handle.events()

                async def wait_for_renewal():
                    async for event in events:
                        if event.kind == "renewed":
                            return event

                renewed = await asyncio.wait_for(wait_for_renewal(), timeout=3)
                self.assertEqual(renewed.requested_lifetime, 2)
                self.assertEqual(renewed.renew_after_ms, 1000)
                self.assertIn(
                    handle.last_event.kind,
                    {"renewed", "notification_observed", "impending_expiry"},
                )

                await handle.close()
                self.assertTrue(handle.closed)
                await handle.cancel()  # idempotent
        finally:
            transport.close()

    async def test_managed_cov_surfaces_renewal_failure_and_can_cancel(self) -> None:
        responder = _ManagedCovFailureResponder()
        transport, _ = await asyncio.get_running_loop().create_datagram_endpoint(
            lambda: responder,
            local_addr=("127.0.0.1", 0),
        )
        address = "{}:{}".format(*transport.get_extra_info("sockname"))
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=50,
            ) as client:
                handle = await client.manage_cov_subscription(
                    address,
                    subscriber_process_identifier=44,
                    monitored_object_identifier=analog_output,
                    confirmed=True,
                    lifetime=1,
                    renewal_margin_ms=500,
                )
                events = handle.events()

                async def wait_for_failure():
                    async for event in events:
                        if event.kind == "renewal_failed":
                            return event

                failed = await asyncio.wait_for(wait_for_failure(), timeout=2)
                self.assertIn("timed out", failed.error)
                self.assertTrue(handle.finished)
                self.assertEqual(handle.last_event.kind, "renewal_failed")
                await handle.cancel()
                self.assertTrue(handle.closed)
                self.assertGreaterEqual(responder.request_count, 3)
                self.assertEqual(responder.cancellation_count, 1)
        finally:
            transport.close()

    async def test_client_stop_closes_managed_cov_and_releases_socket(self) -> None:
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind(("127.0.0.1", 0))
            client_port = probe.getsockname()[1]

        client = BACnetClient(
            interface="127.0.0.1",
            port=client_port,
            broadcast_address="127.0.0.1",
        )
        await client.__aenter__()
        handle = await client.manage_cov_subscription(
            self.address,
            subscriber_process_identifier=46,
            monitored_object_identifier=analog_output,
            confirmed=False,
            lifetime=60,
            renewal_margin_ms=30_000,
        )
        await client.stop()
        self.assertTrue(handle.closed)
        await handle.cancel()

        async with BACnetClient(
            interface="127.0.0.1",
            port=client_port,
            broadcast_address="127.0.0.1",
        ) as rebound:
            value = await rebound.read_property(
                self.address,
                analog_output,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertEqual(value.tag, "real")

    async def test_protocol_error_is_typed_and_client_recovers(self) -> None:
        missing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 99_999)
        existing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            with self.assertRaises(BacnetProtocolError) as raised:
                await client.read_property(
                    self.address,
                    missing_object,
                    PropertyIdentifier.PRESENT_VALUE,
                )

            self.assertEqual(raised.exception.error_class, 1)
            self.assertEqual(raised.exception.error_code, 31)

            recovered = await client.read_property(
                self.address,
                existing_object,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertAlmostEqual(recovered.value, 21.5)

    async def test_timeout_is_typed_and_client_recovers(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind(("127.0.0.1", 0))
            unused_address = f"127.0.0.1:{probe.getsockname()[1]}"

        existing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_timeout_ms=50,
        ) as client:
            with self.assertRaises(BacnetTimeoutError):
                await client.read_property(
                    unused_address,
                    existing_object,
                    PropertyIdentifier.PRESENT_VALUE,
                )

            recovered = await client.read_property(
                self.address,
                existing_object,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertAlmostEqual(recovered.value, 21.5)

    async def test_reject_and_abort_are_typed_and_client_recovers(self) -> None:
        existing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        loop = asyncio.get_running_loop()

        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_timeout_ms=250,
        ) as client:
            for kind, error_type, expected_reason in (
                ("reject", BacnetRejectError, 4),
                ("abort", BacnetAbortError, 1),
            ):
                protocol = _FaultResponder(kind)
                transport, _ = await loop.create_datagram_endpoint(
                    lambda: protocol,
                    local_addr=("127.0.0.1", 0),
                )
                fault_address = "{}:{}".format(*transport.get_extra_info("sockname"))
                try:
                    with self.assertRaises(error_type) as raised:
                        await client.read_property(
                            fault_address,
                            existing_object,
                            PropertyIdentifier.PRESENT_VALUE,
                        )
                    self.assertEqual(raised.exception.reason, expected_reason)
                    self.assertTrue(protocol.received.is_set())
                finally:
                    transport.close()

                recovered = await client.read_property(
                    self.address,
                    existing_object,
                    PropertyIdentifier.PRESENT_VALUE,
                )
                self.assertAlmostEqual(recovered.value, 21.5)

    async def test_malformed_response_times_out_and_client_recovers(self) -> None:
        existing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        loop = asyncio.get_running_loop()
        protocol = _FaultResponder("malformed")
        transport, _ = await loop.create_datagram_endpoint(
            lambda: protocol,
            local_addr=("127.0.0.1", 0),
        )
        fault_address = "{}:{}".format(*transport.get_extra_info("sockname"))

        try:
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=50,
            ) as client:
                with self.assertRaises(BacnetTimeoutError):
                    await client.read_property(
                        fault_address,
                        existing_object,
                        PropertyIdentifier.PRESENT_VALUE,
                    )
                self.assertTrue(protocol.received.is_set())

                recovered = await client.read_property(
                    self.address,
                    existing_object,
                    PropertyIdentifier.PRESENT_VALUE,
                )
                self.assertAlmostEqual(recovered.value, 21.5)
        finally:
            transport.close()

    async def test_transport_bind_failure_is_typed_and_next_client_works(self) -> None:
        occupied_port = int(self.address.rsplit(":", 1)[1])
        conflicting_client = BACnetClient(
            interface="127.0.0.1",
            port=occupied_port,
            broadcast_address="127.0.0.1",
        )

        with self.assertRaises(BacnetError) as raised:
            await conflicting_client.__aenter__()
        self.assertIn("transport error", str(raised.exception).lower())

        existing_object = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        ) as client:
            recovered = await client.read_property(
                self.address,
                existing_object,
                PropertyIdentifier.PRESENT_VALUE,
            )
            self.assertAlmostEqual(recovered.value, 21.5)
