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
EXPECTED_ORIGIN = os.environ.get("W12_EXPECTED_ORIGIN")
COORD = Path(os.environ.get("W12_COORD_DIR", "/coord"))
observed: list[tuple[int, str, str | None]] = []


def policy(context: BvllPolicyContext) -> BvllPolicyVerdict:
    observed.append((context.function, context.udp_sender, context.claimed_origin))
    return BvllPolicyVerdict.allow()


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
        matches = [item for item in observed if item[0] == function and item[1] == EXPECTED_SENDER]
        if EXPECTED_ORIGIN is not None:
            matches = [item for item in matches if item[2] == EXPECTED_ORIGIN]
        if len(matches) != 1:
            raise AssertionError(
                f"BBMD {SIDE} provenance count {len(matches)} != 1 "
                f"for function=0x{function:02x} "
                f"sender={EXPECTED_SENDER} origin={EXPECTED_ORIGIN}: {observed!r}"
            )
        counters = bbmd.bbmd_control.counters()
        if counters.fanout_failure != 0 or counters.fanout_success < 1:
            raise AssertionError(f"BBMD {SIDE} fanout counters: {counters!r}")
        origin = matches[0][2] if matches[0][2] is not None else "none"
        print(
            f"W12_BBMD_PROVENANCE_PASS side={SIDE} function=0x{function:02x} "
            f"udp_sender={matches[0][1]} claimed_origin={origin} "
            "relevant_packets=1 fanout=successful fanout_failure=0",
            flush=True,
        )
        await wait_for(COORD / "complete", timeout=60.0)


if __name__ == "__main__":
    asyncio.run(main())
