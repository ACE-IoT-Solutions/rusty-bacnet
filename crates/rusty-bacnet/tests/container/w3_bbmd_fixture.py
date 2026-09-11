"""Installed-wheel Rust BBMD actor and policy acceptance endpoint."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

from rusty_bacnet import BACnetServer, BvllPolicyContext


SIDE = os.environ["W3_SIDE"]
INTERFACE = os.environ["BACNET_INTERFACE"]
BROADCAST = os.environ["BACNET_BROADCAST"]
PORT = int(os.environ.get("BACNET_PORT", "47808"))
PEER_IP = os.environ["BACNET_BBMD_PEER"]
PEER_PORT = int(os.environ.get("BACNET_BBMD_PEER_PORT", "47808"))
REPORT_PATH = Path("/tmp/w3-report")
ARM_EXPIRY_PATH = Path("/tmp/w3-arm-expiry")
CHECK_EXPIRY_PATH = Path("/tmp/w3-check-expiry")

ALLOW_A = b"\x01\x00W3:ALLOW:A"
ALLOW_B = b"\x01\x00W3:ALLOW:B"
CUT_THROUGH = b"\x01\x00W3:CUT"
DROP = b"\x01\x00W3:DROP"


def emit(message: str) -> None:
    print(message, flush=True)


def policy(context: BvllPolicyContext) -> object:
    if SIDE != "A":
        return "allow"
    if context.function == 0x05 and context.body == b"\x00\x0d":
        return ("reject", 0x0030)
    if context.function == 0x09 and context.npdu == DROP:
        return "drop"
    if context.function == 0x09 and context.npdu == CUT_THROUGH:
        return "allow_and_forward"
    return "allow"


def validate_counters(control) -> None:
    counters = control.counters()
    bridge = control.policy_counters()
    if counters.fanout_failure != 0:
        raise AssertionError(f"side {SIDE} had fanout failures: {counters.fanout_failure}")
    if counters.fanout_success < 2:
        raise AssertionError(f"side {SIDE} did not exercise successful fanout: {counters.fanout_success}")
    if counters.policy_errors != 0 or counters.policy_invalid_verdicts != 0:
        raise AssertionError(
            f"side {SIDE} policy errors={counters.policy_errors} "
            f"invalid={counters.policy_invalid_verdicts}"
        )
    if bridge.exceptions or bridge.timeouts or bridge.overloads or bridge.invalid_verdicts:
        raise AssertionError(
            f"side {SIDE} policy bridge failed closed unexpectedly: "
            f"exceptions={bridge.exceptions} timeouts={bridge.timeouts} "
            f"overloads={bridge.overloads} invalid={bridge.invalid_verdicts}"
        )

    if SIDE == "A":
        dbtn = counters.policy_by_function[0x09]
        register = counters.policy_by_function[0x05]
        if counters.policy_dropped != 1 or dbtn.dropped != 1:
            raise AssertionError(
                f"drop decision was not counted exactly once: total={counters.policy_dropped} "
                f"dbtn={dbtn.dropped}"
            )
        if counters.policy_rejected != 1 or register.rejected != 1:
            raise AssertionError(
                f"registration rejection was not counted exactly once: "
                f"total={counters.policy_rejected} register={register.rejected}"
            )
        if counters.policy_cutthrough != 1 or dbtn.cutthrough != 1:
            raise AssertionError(
                f"cut-through was not counted exactly once: total={counters.policy_cutthrough} "
                f"dbtn={dbtn.cutthrough}"
            )
        if bridge.dropped != 1 or bridge.rejected != 1 or bridge.cutthrough != 1:
            raise AssertionError(
                f"Python bridge decision totals mismatch: dropped={bridge.dropped} "
                f"rejected={bridge.rejected} cutthrough={bridge.cutthrough}"
            )


    emit(
        f"BBMD_COUNTERS side={SIDE} decoded={counters.decoded} "
        f"processed={counters.terminal_processed} fanout_success={counters.fanout_success} "
        f"allowed={counters.policy_allowed} dropped={counters.policy_dropped} "
        f"rejected={counters.policy_rejected} cutthrough={counters.policy_cutthrough}"
    )


async def main() -> None:
    server = BACnetServer(
        device_instance=4_193_100 + (0 if SIDE == "A" else 1),
        interface=INTERFACE,
        port=PORT,
        broadcast_address=BROADCAST,
        bbmd=True,
        bbmd_bdt=[(PEER_IP, PEER_PORT, "255.255.255.255")],
        bbmd_accept_foreign_devices=True,
        bbmd_max_fdt_entries=8,
        bbmd_wire_management_enabled=False,
        bvll_policy=policy,
        bvll_policy_timeout_ms=100,
    )
    await server.start()
    control = server.bbmd_control
    emit(f"BBMD_READY side={SIDE} address={await server.local_address()} peer={PEER_IP}:{PEER_PORT}")

    saw_fdt = False
    reported_expiry = False
    expiry_revision = None
    try:
        async with asyncio.timeout(90):
            while True:
                snapshot = None
                if not saw_fdt:
                    snapshot = await control.snapshot()
                if snapshot is not None and snapshot.fdt and not saw_fdt:
                    saw_fdt = True
                    if len(snapshot.fdt) != 1:
                        raise AssertionError(f"side {SIDE} expected one initial FDT entry: {snapshot.fdt!r}")
                    entry = snapshot.fdt[0]
                    emit(
                        f"BBMD_FDT_ACTIVE side={SIDE} ip={entry.ip} port={entry.port} "
                        f"ttl={entry.ttl} seconds_remaining={entry.seconds_remaining}"
                    )

                if ARM_EXPIRY_PATH.exists():
                    ARM_EXPIRY_PATH.unlink()
                    snapshot = await control.snapshot()
                    if SIDE != "B" or not snapshot.fdt:
                        raise AssertionError(f"invalid expiry arm state on side {SIDE}: {snapshot.fdt!r}")
                    expiry_revision = snapshot.revision
                    emit(f"BBMD_FDT_EXPIRY_ARMED side={SIDE} revision={expiry_revision}")

                if CHECK_EXPIRY_PATH.exists():
                    CHECK_EXPIRY_PATH.unlink()
                    snapshot = await control.snapshot()
                    if SIDE != "B" or snapshot.fdt:
                        raise AssertionError(f"side {SIDE} FDT did not expire: {snapshot.fdt!r}")
                    if expiry_revision is None or snapshot.revision <= expiry_revision:
                        raise AssertionError(
                            f"actor expiry revision did not advance: armed={expiry_revision} "
                            f"current={snapshot.revision}"
                        )
                    reported_expiry = True
                    emit(f"BBMD_FDT_EXPIRED side={SIDE} revision={snapshot.revision}")

                if REPORT_PATH.exists():
                    REPORT_PATH.unlink()
                    if SIDE == "B" and not reported_expiry:
                        raise AssertionError("side B report requested before FDT expiry was observable")
                    snapshot = await control.snapshot()
                    if SIDE == "A" and any(entry.ttl == 13 for entry in snapshot.fdt):
                        raise AssertionError("rejected TTL-13 probe mutated side A FDT")
                    validate_counters(control)
                await asyncio.sleep(0.1)
    finally:
        await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
