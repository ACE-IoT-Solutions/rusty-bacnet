"""Installed-wheel BBMD endpoint for the standards-correct W12 campus fixture."""

from __future__ import annotations

import asyncio
import os
import subprocess
from pathlib import Path

from rusty_bacnet import BACnetClient, BvllPolicyContext, BvllPolicyVerdict


SIDE = os.environ["W12_SIDE"]
INTERFACE = os.environ["BACNET_INTERFACE"]
BROADCAST = os.environ["BACNET_BROADCAST"]
PEER = os.environ["BACNET_BBMD_PEER"]
REMOTE_SUBNET = os.environ["W12_REMOTE_SUBNET"]
GATEWAY = os.environ["W12_GATEWAY"]
EXPECTED_SENDER = os.environ["W12_EXPECTED_SENDER"]
COORD = Path(os.environ.get("W12_COORD_DIR", "/coord"))
observed: list[tuple[int, str]] = []


def policy(context: BvllPolicyContext) -> BvllPolicyVerdict:
    observed.append((context.function, f"{context.source_ip}:{context.source_port}"))
    return BvllPolicyVerdict.continue_native()


async def wait_for(path: Path, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.02)


async def main() -> None:
    subprocess.run(
        ["ip", "route", "replace", REMOTE_SUBNET, "via", GATEWAY], check=True
    )
    bbmd = BACnetClient(
        interface=INTERFACE,
        port=47808,
        broadcast_address=BROADCAST,
        bbmd=True,
        bbmd_bdt=[(PEER, 47808, "255.255.255.255")],
        bvll_policy=policy,
    )
    async with bbmd:
        print(f"W12_BBMD_READY side={SIDE} address={INTERFACE}:47808 peer={PEER}:47808", flush=True)
        await wait_for(COORD / "discovery.done")
        function = 0x0B if SIDE == "A" else 0x04
        matches = [item for item in observed if item == (function, EXPECTED_SENDER)]
        if len(matches) != 1:
            raise AssertionError(
                f"BBMD {SIDE} source count {len(matches)} != 1 "
                f"for function=0x{function:02x} sender={EXPECTED_SENDER}: {observed!r}"
            )
        snapshot = await bbmd.bbmd_control.snapshot()
        counters = snapshot.counters
        if counters.fanout_send_errors != 0 or counters.fanout_packets_forwarded < 1:
            raise AssertionError(f"BBMD {SIDE} fanout counters: {counters!r}")
        print(
            f"W12_BBMD_SOURCE_PASS side={SIDE} function=0x{function:02x} "
            f"udp_sender={matches[0][1]} relevant_packets=1 "
            "fanout=successful fanout_send_errors=0",
            flush=True,
        )
        await wait_for(COORD / "complete", timeout=60.0)


if __name__ == "__main__":
    asyncio.run(main())
