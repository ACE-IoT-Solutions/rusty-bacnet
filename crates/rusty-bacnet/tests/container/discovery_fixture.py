"""Two-container installed-wheel Who-Is/I-Am fixture."""

from __future__ import annotations

import asyncio
import os
import socket
import sys

from rusty_bacnet import BACnetClient, BACnetServer


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

        expected_routers = {
            "172.31.0.11:47808": [1000, 2001, 3000],
            "172.31.0.12:47808": [4000, 5000],
        }
        for _ in range(5):
            unscoped = await client.who_is_router(timeout_ms=400)
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

        scoped = await client.who_is_router(network=2001, timeout_ms=400)
        scoped_actual = {router.address: router.networks for router in scoped}
        if scoped_actual != {"172.31.0.11:47808": [1000, 2001, 3000]}:
            raise AssertionError(f"scoped router claims mismatch: {scoped_actual!r}")

        unknown = await client.who_is_router(network=65000, timeout_ms=200)
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

        if len(fdt) != 1:
            raise AssertionError(f"unexpected bacpypes3 FDT: {fdt!r}")
        foreign = fdt[0]
        if (
            foreign.ip_bytes != bytes((172, 31, 0, 32))
            or foreign.port != 47810
            or foreign.ttl != 120
            or not 1 <= foreign.seconds_remaining <= 125
        ):
            raise AssertionError(f"bacpypes3 FDT fields mismatch: {foreign!r}")
        first_remaining = foreign.seconds_remaining
        await asyncio.sleep(2)
        later_fdt = await client.read_fdt("172.31.0.30:47809", timeout_ms=500)
        if len(later_fdt) != 1 or later_fdt[0].seconds_remaining >= first_remaining:
            raise AssertionError(
                "bacpypes3 FDT remaining time did not decrease: "
                f"{first_remaining} -> {later_fdt!r}"
            )


class _RouterResponder(asyncio.DatagramProtocol):
    def __init__(self, claims: list[list[int]]) -> None:
        self.claims = claims

    def connection_made(self, transport) -> None:
        self.transport = transport
        sock = transport.get_extra_info("socket")
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)

    def datagram_received(self, data: bytes, _address) -> None:
        if len(data) < 7 or data[:2] != b"\x81\x0b":
            return
        npdu = data[4:]
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
