"""Two-container installed-wheel Who-Is/I-Am fixture."""

from __future__ import annotations

import asyncio
import os
import socket
import sys

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    RoutedTarget,
)


INTERFACE = os.environ["BACNET_INTERFACE"]
BROADCAST = os.environ["BACNET_BROADCAST"]
PORT = int(os.environ.get("BACNET_PORT", "47808"))
DEVICE_INSTANCE = int(os.environ["BACNET_DEVICE_INSTANCE"])


async def run_server() -> None:
    server = BACnetServer(
        device_instance=DEVICE_INSTANCE,
        device_name="Installed Wheel Discovery Fixture",
        interface=INTERFACE,
        port=PORT,
        broadcast_address=BROADCAST,
    )
    server.add_analog_input(
        instance=1,
        name="Fixture Temperature",
        units=62,
        present_value=21.5,
    )
    server.add_analog_output(
        instance=1,
        name="Fixture Setpoint",
        units=62,
    )
    server.add_character_string_value(
        instance=1,
        name="Fixture Long Text",
    )
    await server.start()
    try:
        await asyncio.Event().wait()
    finally:
        await server.stop()


async def run_client() -> None:
    expected_mac = bytes.fromhex(os.environ["BACNET_EXPECTED_MAC_HEX"])

    async with BACnetClient(
        interface=INTERFACE,
        port=PORT,
        broadcast_address=BROADCAST,
        apdu_timeout_ms=500,
    ) as client:
        for _ in range(5):
            devices = await client.discover(
                timeout_ms=500,
                low_limit=DEVICE_INSTANCE,
                high_limit=DEVICE_INSTANCE,
            )
            matching = [
                device
                for device in devices
                if device.object_identifier.instance == DEVICE_INSTANCE
            ]
            if matching:
                device = matching[0]
                if device.mac_address != expected_mac:
                    raise AssertionError(
                        f"I-Am MAC mismatch: {device.mac_address.hex()} != "
                        f"{expected_mac.hex()}"
                    )
                if device.source_network is not None:
                    raise AssertionError(
                        f"direct I-Am unexpectedly routed via {device.source_network}"
                    )
                break
            await asyncio.sleep(0.25)

        else:
            raise AssertionError(
                f"device {DEVICE_INSTANCE} was not discovered after five Who-Is attempts"
            )

        analog_input = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        analog_output = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
        direct_value = await client.read_property(
            "172.31.0.10:47808", analog_input, PropertyIdentifier.PRESENT_VALUE
        )
        if round(direct_value.value, 1) != 21.5:
            raise AssertionError(f"direct B/IP read mismatch: {direct_value!r}")
        await client.write_property(
            "172.31.0.10:47808",
            analog_output,
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(18.5),
            priority=8,
        )

        routed = RoutedTarget("172.31.0.11:47808", 2001, b"\x00\x2a")
        routed_value = await client.read_property(
            routed, analog_input, PropertyIdentifier.PRESENT_VALUE
        )
        if round(routed_value.value, 1) != 21.5:
            raise AssertionError(f"routed B/IP read mismatch: {routed_value!r}")
        await client.write_property(
            routed,
            analog_output,
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(19.5),
            priority=8,
        )
        routed_written = await client.read_property(
            routed, analog_output, PropertyIdentifier.PRESENT_VALUE
        )
        if round(routed_written.value, 1) != 19.5:
            raise AssertionError(f"routed B/IP write mismatch: {routed_written!r}")

        expected_routers = {
            "172.31.0.11:47808": [1000, 2001, 3000],
            "172.31.0.12:47808": [4000, 5000],
        }
        for _ in range(5):
            unscoped = await client.who_is_router_to_network(observation_window_ms=400)
            actual = {router.address: router.networks for router in unscoped}
            if actual == expected_routers:
                break
            await asyncio.sleep(0.25)
        else:
            raise AssertionError(f"unscoped router claims mismatch: {actual!r}")
        for router in unscoped:
            if router.mac_address[-2:] != PORT.to_bytes(2, "big"):
                raise AssertionError(
                    f"router port was not preserved: {router.mac_address.hex()}"
                )
            try:
                router.networks = []
            except AttributeError:
                pass
            else:
                raise AssertionError("RouterInfo must be immutable")

        scoped = await client.who_is_router_to_network(network=2001, observation_window_ms=400)
        scoped_actual = {router.address: router.networks for router in scoped}
        if scoped_actual != {"172.31.0.11:47808": [1000, 2001, 3000]}:
            raise AssertionError(f"scoped router claims mismatch: {scoped_actual!r}")

        unknown = await client.who_is_router_to_network(network=65000, observation_window_ms=200)
        if unknown:
            raise AssertionError(f"unknown scoped network returned routers: {unknown!r}")

        for _ in range(5):
            try:
                bdt = await client.read_bdt("172.31.0.30:47809", timeout_ms=500)
                fdt = await client.read_fdt("172.31.0.30:47809", timeout_ms=500)
                break
            except Exception:
                await asyncio.sleep(0.25)
        else:
            raise AssertionError("bacpypes3 BBMD did not become ready")

        if len(bdt) != 1:
            raise AssertionError(f"unexpected bacpypes3 BDT: {bdt!r}")
        peer = bdt[0]
        if (
            peer.ip_bytes != bytes((172, 31, 0, 31))
            or peer.port != 47811
            or peer.broadcast_mask_bytes != b"\xff\xff\xff\x00"
        ):
            raise AssertionError(f"bacpypes3 BDT fields mismatch: {peer!r}")

        if fdt:
            raise AssertionError(
                "bacpypes3 FDT must start empty; entries require live registration: "
                f"{fdt!r}"
            )


class _RouterResponder(asyncio.DatagramProtocol):
    def __init__(self, claims: list[list[int]]) -> None:
        self.claims = claims
        self.routed_server = os.environ.get("BACNET_ROUTED_SERVER")
        self.routed_network = int(os.environ.get("BACNET_ROUTED_NETWORK", "0"))
        self.routed_dadr = bytes.fromhex(os.environ.get("BACNET_ROUTED_DADR_HEX", ""))
        self.routed_client = None

    def connection_made(self, transport) -> None:
        self.transport = transport
        sock = transport.get_extra_info("socket")
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)

    def datagram_received(self, data: bytes, address) -> None:
        if len(data) < 6 or data[0] != 0x81:
            return
        npdu = data[4:]
        if self.routed_server and data[1] == 0x0A and npdu[1] & 0x20:
            offset = 2
            network = int.from_bytes(npdu[offset : offset + 2], "big")
            offset += 2
            dlen = npdu[offset]
            offset += 1
            dadr = npdu[offset : offset + dlen]
            offset += dlen + 1
            if network != self.routed_network or dadr != self.routed_dadr:
                raise AssertionError(
                    f"routed target mismatch: {network}/{dadr.hex()}"
                )
            self.routed_client = address
            direct_npdu = b"\x01\x04" + npdu[offset:]
            frame = b"\x81\x0a" + (len(direct_npdu) + 4).to_bytes(2, "big")
            host, port = self.routed_server.rsplit(":", 1)
            self.transport.sendto(frame + direct_npdu, (host, int(port)))
            return
        if (
            self.routed_server
            and data[1] == 0x0A
            and address[0] == self.routed_server.rsplit(":", 1)[0]
            and self.routed_client is not None
        ):
            routed_npdu = (
                b"\x01\x08"
                + self.routed_network.to_bytes(2, "big")
                + bytes((len(self.routed_dadr),))
                + self.routed_dadr
                + npdu[2:]
            )
            frame = b"\x81\x0a" + (len(routed_npdu) + 4).to_bytes(2, "big")
            self.transport.sendto(frame + routed_npdu, self.routed_client)
            return
        if data[1] != 0x0B or len(npdu) < 3:
            return
        if npdu[:3] != b"\x01\x80\x00":
            return
        requested = (
            int.from_bytes(npdu[3:5], "big") if len(npdu) >= 5 else None
        )
        for networks in self.claims:
            if requested is not None and requested not in networks:
                continue
            payload = b"".join(network.to_bytes(2, "big") for network in networks)
            response_npdu = b"\x01\x80\x01" + payload
            frame = (
                b"\x81\x0b"
                + (len(response_npdu) + 4).to_bytes(2, "big")
                + response_npdu
            )
            self.transport.sendto(frame, (BROADCAST, PORT))


async def run_router() -> None:
    claims = [
        [int(network) for network in group.split(",")]
        for group in os.environ["BACNET_ROUTER_CLAIMS"].split(";")
    ]
    await asyncio.get_running_loop().create_datagram_endpoint(
        lambda: _RouterResponder(claims),
        # Bind INADDR_ANY so Linux delivers directed-broadcast datagrams.
        local_addr=("0.0.0.0", PORT),
    )
    await asyncio.Event().wait()


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in {"server", "client", "router"}:
        raise SystemExit("usage: discovery_fixture.py {server|client|router}")
    action = {"server": run_server, "client": run_client, "router": run_router}
    asyncio.run(action[sys.argv[1]]())


if __name__ == "__main__":
    main()
