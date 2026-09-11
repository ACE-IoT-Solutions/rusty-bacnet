"""Pinned bacpypes3 scanner for W5 external virtual-router acceptance."""

from __future__ import annotations

import asyncio
import os
from argparse import Namespace
from pathlib import Path

from bacpypes3.app import Application
from bacpypes3.pdu import Address, IPv4Address
from bacpypes3.primitivedata import ObjectIdentifier


LOCAL_ADDRESS = os.environ.get("BACNET_LOCAL_ADDRESS", "10.254.51.20/24:47808")
ROUTER_ADDRESS = os.environ.get("BACNET_ROUTER_ADDRESS", "10.254.51.10:47808")
COORD = Path(os.environ.get("W5_COORD_DIR", "/coord"))
EXPECTED = (
    (1201, 10, 4150501, 21.25),
    (1202, 11, 4150502, 22.5),
)


async def wait_for_file(path: Path, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.05)


async def discover_router(app: Application, phase: int) -> None:
    results = await app.nse.who_is_router_to_network()
    matching = [
        npdu
        for _adapter, npdu in results
        if npdu.pduSource == IPv4Address(ROUTER_ADDRESS)
    ]
    if len(matching) != 1:
        raise AssertionError(f"phase {phase}: router responses mismatch: {results!r}")
    networks = sorted(matching[0].iartnNetworkList)
    if networks != [1201, 1202]:
        raise AssertionError(f"phase {phase}: IARTN networks mismatch: {networks!r}")
    print(
        f"W5_IARTN_PASS phase={phase} router={ROUTER_ADDRESS} networks=1201,1202",
        flush=True,
    )


async def discover_and_read(app: Application, phase: int) -> None:
    for network, mac, instance, expected_value in EXPECTED:
        responses = await app.who_is(
            low_limit=instance,
            high_limit=instance,
            address=Address(f"{network}:*"),
            timeout=2,
        )
        if len(responses) != 1:
            raise AssertionError(
                f"phase {phase}: device {instance} discovery mismatch: {responses!r}"
            )
        source = responses[0].pduSource
        if source.addrNet != network or source.addrAddr != bytes((mac,)):
            raise AssertionError(
                f"phase {phase}: source mismatch for {instance}: "
                f"network={source.addrNet!r} address={source.addrAddr!r}"
            )
        if responses[0].iAmDeviceIdentifier != ObjectIdentifier(("device", instance)):
            raise AssertionError(
                f"phase {phase}: I-Am device identifier mismatch: "
                f"{responses[0].iAmDeviceIdentifier!r}"
            )

        target = Address(f"{network}:{mac}")
        value = await app.read_property(
            target,
            ObjectIdentifier("analog-input,1"),
            "present-value",
        )
        if round(float(value), 2) != expected_value:
            raise AssertionError(
                f"phase {phase}: read {network}:{mac}={value!r}, "
                f"expected {expected_value}"
            )
        print(
            f"W5_ROUTED_DEVICE_PASS phase={phase} instance={instance} "
            f"dnet={network} dadr={mac:02x} snet={source.addrNet} "
            f"sadr={source.addrAddr.hex()} value={float(value):.2f}",
            flush=True,
        )


async def main() -> None:
    args = Namespace(
        name="W5 bacpypes3 routed scanner",
        instance=4150599,
        network=100,
        address=LOCAL_ADDRESS,
        vendoridentifier=999,
        foreign=None,
        ttl=30,
        bbmd=None,
    )
    app = Application.from_args(args)
    try:
        await discover_router(app, phase=1)
        await discover_and_read(app, phase=1)
        (COORD / "phase1.done").touch()

        await wait_for_file(COORD / "router.restarted")
        await discover_router(app, phase=2)
        await discover_and_read(app, phase=2)
        print(
            "W5_VIRTUAL_ROUTER_ACCEPTANCE_PASS phases=2 networks=2 devices=2 "
            "reads=4 source_addresses=4 restart=pass",
            flush=True,
        )
    finally:
        app.close()


if __name__ == "__main__":
    asyncio.run(main())
