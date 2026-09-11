"""Remote B/IP router plus IP-less virtual BACnet device for W12."""

from __future__ import annotations

import asyncio
import os
import subprocess
from pathlib import Path

from rusty_bacnet import (
    BACnetRouter,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


INTERFACE = os.environ.get("BACNET_INTERFACE", "10.254.122.20")
BROADCAST = os.environ.get("BACNET_BROADCAST", "10.254.122.255")
REMOTE_SUBNET = os.environ.get("W12_REMOTE_SUBNET", "10.254.121.0/24")
GATEWAY = os.environ.get("W12_GATEWAY", "10.254.122.254")
COORD = Path(os.environ.get("W12_COORD_DIR", "/coord"))
VIRTUAL_NETWORK = "w12-remote-device-lan"
DNET = 2120
DEVICE_MAC = 0x2A
DEVICE_INSTANCE = 4121200


async def wait_for(path: Path, timeout: float = 60.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.02)


async def main() -> None:
    subprocess.run(
        ["ip", "route", "replace", REMOTE_SUBNET, "via", GATEWAY], check=True
    )
    device = BACnetServer(
        DEVICE_INSTANCE,
        "W12 remote virtual device",
        transport="virtual",
        virtual_network=VIRTUAL_NETWORK,
        virtual_mac=DEVICE_MAC,
    )
    device.add_analog_value(
        1,
        "W12 routed COV value",
        units=62,
        present_value=20.0,
        cov_increment=0.5,
    )
    router = BACnetRouter(
        bip_network=1220,
        virtual_ports=[(DNET, VIRTUAL_NETWORK, 1)],
        interface=INTERFACE,
        port=47808,
        broadcast_address=BROADCAST,
    )
    await device.start()
    await router.start()
    print(
        "W12_REMOTE_READY "
        f"router={INTERFACE}:47808 bip_network=1220 "
        f"device_transport=virtual dnet={DNET} dadr={DEVICE_MAC:02x} ip_endpoint=none",
        flush=True,
    )
    try:
        await wait_for(COORD / "cov.ready")
        await device.write_property_local(
            ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(21.5),
            priority=8,
        )
        print("W12_REMOTE_COV_UPDATE value=21.5", flush=True)
        await wait_for(COORD / "complete")
    finally:
        await router.stop()
        await device.stop()


if __name__ == "__main__":
    asyncio.run(main())
