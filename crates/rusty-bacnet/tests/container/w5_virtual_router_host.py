"""Installed-wheel W5 host: one B/IP router and two virtual BACnet networks."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

from rusty_bacnet import BACnetRouter, BACnetServer


INTERFACE = os.environ.get("BACNET_INTERFACE", "10.254.51.10")
BROADCAST = os.environ.get("BACNET_BROADCAST", "10.254.51.255")
PORT = int(os.environ.get("BACNET_PORT", "47808"))
COORD = Path(os.environ.get("W5_COORD_DIR", "/coord"))
NETWORKS = (1201, 1202)
NAMES = ("w5-virtual-a", "w5-virtual-b")
SERVER_MACS = (10, 11)
ROUTER_MAC = 1
INSTANCES = (4150501, 4150502)
VALUES = (21.25, 22.5)


async def wait_for_file(path: Path, timeout: float = 60.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.05)


async def main() -> None:
    COORD.mkdir(parents=True, exist_ok=True)
    for marker in ("phase1.done", "router.restarted"):
        (COORD / marker).unlink(missing_ok=True)

    servers = []
    for instance, name, mac, value in zip(INSTANCES, NAMES, SERVER_MACS, VALUES):
        server = BACnetServer(
            device_instance=instance,
            device_name=f"W5 virtual device {instance}",
            transport="virtual",
            virtual_network=name,
            virtual_mac=mac,
        )
        server.add_analog_input(
            instance=1,
            name=f"W5 input {instance}",
            units=62,
            present_value=value,
        )
        servers.append(server)

    router = BACnetRouter(
        bip_network=100,
        virtual_ports=[
            (network, name, ROUTER_MAC) for network, name in zip(NETWORKS, NAMES)
        ],
        interface=INTERFACE,
        port=PORT,
        broadcast_address=BROADCAST,
    )

    started_servers = []
    try:
        for server in servers:
            await server.start()
            started_servers.append(server)
        await router.start()
        address = await router.local_address()
        if address != f"{INTERFACE}:{PORT}":
            raise AssertionError(f"router address mismatch: {address}")
        print(
            "W5_ROUTER_READY "
            f"address={address} networks={NETWORKS[0]},{NETWORKS[1]} "
            f"server_macs={SERVER_MACS[0]:02x},{SERVER_MACS[1]:02x}",
            flush=True,
        )

        await wait_for_file(COORD / "phase1.done")
        await router.stop()
        stopped = await router.port_counters()
        if len(stopped) != 3:
            raise AssertionError(f"expected three stopped counter snapshots: {stopped!r}")

        # The same object rebuilds all terminal transports. Successful start
        # proves both prior virtual memberships and the fixed B/IP socket were
        # released by stop().
        await router.start()
        (COORD / "router.restarted").touch()
        print(
            "W5_ROUTER_RESTARTED memberships_released=2 "
            f"address={await router.local_address()}",
            flush=True,
        )
        await asyncio.Event().wait()
    finally:
        await router.stop()
        for server in reversed(started_servers):
            await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
