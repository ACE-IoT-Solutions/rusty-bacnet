"""Installed-wheel W12 client: BBMD discovery, routed read, and routed COV."""

from __future__ import annotations

import asyncio
import os
import subprocess
from pathlib import Path

from rusty_bacnet import (
    BACnetClient,
    BacnetTimeoutError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    RoutedTarget,
)


INTERFACE = os.environ.get("BACNET_INTERFACE", "10.254.121.20")
BROADCAST = os.environ.get("BACNET_BROADCAST", "10.254.121.255")
REMOTE_SUBNET = os.environ.get("W12_REMOTE_SUBNET", "10.254.122.0/24")
GATEWAY = os.environ.get("W12_GATEWAY", "10.254.121.254")
ROUTER = os.environ.get("BACNET_REMOTE_ROUTER", "10.254.122.20:47808")
COORD = Path(os.environ.get("W12_COORD_DIR", "/coord"))
DNET = 2120
DADR = b"\x2a"
DEVICE_INSTANCE = 4121200
DEVICE = ObjectIdentifier(ObjectType.DEVICE, DEVICE_INSTANCE)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)


async def wait_for(path: Path, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.02)


async def next_matching(stream: object) -> object:
    async with asyncio.timeout(5):
        while True:
            event = await anext(stream)
            if event.device_instance == DEVICE_INSTANCE:
                return event


def present_value(notification: object) -> float:
    values = notification.values
    if len(values) != 2:
        raise AssertionError(f"COV value count {len(values)} != 2")
    if values[0]["property_id"] != PropertyIdentifier.PRESENT_VALUE:
        raise AssertionError(f"first COV property: {values[0]!r}")
    if values[1]["property_id"] != PropertyIdentifier.STATUS_FLAGS:
        raise AssertionError(f"second COV property: {values[1]!r}")
    decoded = values[0]["value"]
    return round(float(decoded.value), 2)


async def main() -> None:
    subprocess.run(
        ["ip", "route", "replace", REMOTE_SUBNET, "via", GATEWAY], check=True
    )
    route = subprocess.run(
        ["ip", "-4", "route", "get", ROUTER.split(":", 1)[0]],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if f"via {GATEWAY}" not in route:
        raise AssertionError(f"remote B/IP route does not use campus router: {route!r}")
    print(
        f"W12_ROUTE_PRECONDITION_PASS remote_bip_route_via={GATEWAY} "
        f"device_datalink=virtual:{DNET}:{DADR.hex()} device_ip_endpoint=none",
        flush=True,
    )

    async with BACnetClient(
        interface=INTERFACE,
        port=47808,
        broadcast_address=BROADCAST,
        apdu_timeout_ms=600,
    ) as client:
        absent = await client.discover(
            timeout_ms=700, low_limit=DEVICE_INSTANCE, high_limit=DEVICE_INSTANCE
        )
        if absent:
            raise AssertionError(f"remote device crossed isolated broadcasts without BBMDs: {absent!r}")
        print("W12_ISOLATED_DISCOVERY_NEGATIVE_PASS bbmds=stopped devices=0", flush=True)
        (COORD / "negative.done").touch()
        await wait_for(COORD / "bbmd.ready")

        stream = await client.who_is_stream(
            low_limit=DEVICE_INSTANCE, high_limit=DEVICE_INSTANCE
        )
        event = await next_matching(stream)
        expected_mac = bytes((10, 254, 122, 20, 0xBA, 0xC0))
        if event.source_network != DNET or event.source_address != DADR:
            raise AssertionError(
                f"I-Am routed source {(event.source_network, event.source_address)!r}"
            )
        if event.source_mac != expected_mac:
            raise AssertionError(f"I-Am data-link source {event.source_mac.hex()}")
        if event.udp_source != ROUTER or event.bvlc_function != 0x0A:
            raise AssertionError(
                f"I-Am transport provenance udp={event.udp_source!r} "
                f"bvlc={event.bvlc_function!r} forwarded={event.forwarded_from!r}"
            )
        if event.forwarded_from is not None:
            raise AssertionError(f"unicast I-Am unexpectedly forwarded: {event.forwarded_from}")
        print(
            f"W12_BBMD_DISCOVERY_PASS device={DEVICE_INSTANCE} snet={DNET} "
            f"sadr={DADR.hex()} udp_source={event.udp_source} bvlc=0x0a "
            "reply_path=direct-per-annex-j",
            flush=True,
        )
        (COORD / "discovery.done").touch()

        # The router's IP is not a fallback endpoint for the virtual device.
        try:
            await client.read_property(ROUTER, DEVICE, PropertyIdentifier.OBJECT_NAME)
        except BacnetTimeoutError:
            pass
        else:
            raise AssertionError("virtual device unexpectedly answered as a direct B/IP endpoint")

        target = RoutedTarget(ROUTER, DNET, DADR)
        value = await client.read_property(target, AV, PropertyIdentifier.PRESENT_VALUE)
        if round(float(value.value), 2) != 20.0:
            raise AssertionError(f"routed AV read mismatch: {value!r}")
        print(
            f"W12_ROUTED_READ_PASS router={ROUTER} dnet={DNET} dadr={DADR.hex()} "
            "value=20.0 direct_device_endpoint=absent",
            flush=True,
        )

        notifications = await client.cov_notifications()
        await client.subscribe_cov(target, 12001, AV, confirmed=True, lifetime=30)
        async with asyncio.timeout(5):
            initial = await anext(notifications)
        if present_value(initial) != 20.0:
            raise AssertionError(f"initial COV mismatch: {initial!r}")
        (COORD / "cov.ready").touch()
        async with asyncio.timeout(5):
            changed = await anext(notifications)
        if present_value(changed) != 21.5:
            raise AssertionError(f"changed COV mismatch: {changed!r}")
        if changed.delivery != "confirmed":
            raise AssertionError(f"COV delivery is {changed.delivery!r}")
        if changed.source_network != DNET or changed.source_address != DADR:
            raise AssertionError(
                f"COV routed source {(changed.source_network, changed.source_address)!r}"
            )
        if changed.source_mac != expected_mac:
            raise AssertionError(f"COV router MAC {changed.source_mac.hex()}")
        print(
            f"W12_CONFIRMED_COV_PASS process=12001 delivery=confirmed "
            f"snet={DNET} sadr={DADR.hex()} router_mac={changed.source_mac.hex()} value=21.5",
            flush=True,
        )
        await client.unsubscribe_cov(target, 12001, AV)

    (COORD / "complete").touch()
    print("W12_ACCEPTANCE_PASS", flush=True)


if __name__ == "__main__":
    asyncio.run(main())
