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
CONTINUE_NATIVE = b"\x01\x00W3:CUT"
DROP = b"\x01\x00W3:DROP"


def emit(message: str) -> None:
    print(message, flush=True)


def policy(context: BvllPolicyContext) -> object:
    if SIDE != "A":
        return "continue"
    payload = bytes(context.payload_prefix)
    if context.function == 0x05 and context.payload_len == 2 and payload == b"\x00\x0d":
        return ("reject", 0x0030)
    if context.function == 0x09 and context.payload_len == len(DROP) and payload == DROP:
        return "drop"
    if (
        context.function == 0x09
        and context.payload_len == len(CONTINUE_NATIVE)
        and payload == CONTINUE_NATIVE
    ):
        return "continue"
    return "continue"


def validate_counters(control, snapshot) -> None:
    counters = snapshot.counters
    bridge = control.policy_counters()
    if counters.fanout_send_errors != 0:
        raise AssertionError(f"side {SIDE} had fanout errors: {counters.fanout_send_errors}")
    if counters.fanout_packets_forwarded < 2:
        raise AssertionError(
            f"side {SIDE} did not exercise successful fanout: "
            f"{counters.fanout_packets_forwarded}"
        )
    if counters.fanout_packets_throttled or counters.fanout_queue_overflow_drops:
        raise AssertionError(
            f"side {SIDE} unexpectedly throttled or dropped fanout: "
            f"throttled={counters.fanout_packets_throttled} "
            f"queue_drops={counters.fanout_queue_overflow_drops}"
        )
    if bridge.errors or bridge.timeouts or bridge.overloads or bridge.circuit_open_drops:
        raise AssertionError(
            f"side {SIDE} policy bridge failed closed unexpectedly: "
            f"errors={bridge.errors} timeouts={bridge.timeouts} "
            f"overloads={bridge.overloads} circuit_open={bridge.circuit_open_drops}"
        )

    if SIDE == "A":
        if bridge.dropped != 1:
            raise AssertionError(
                f"drop decision was not counted exactly once: {bridge.dropped}"
            )
        if bridge.rejected != 1:
            raise AssertionError(
                f"registration rejection was not counted exactly once: {bridge.rejected}"
            )
        if bridge.continued < 1:
            raise AssertionError(
                f"native continuation was not exercised: continued={bridge.continued}"
            )

    emit(
        f"BBMD_COUNTERS side={SIDE} fanout_forwarded={counters.fanout_packets_forwarded} "
        f"fanout_send_errors={counters.fanout_send_errors} "
        f"continued={bridge.continued} dropped={bridge.dropped} "
        f"rejected={bridge.rejected} policy_errors={bridge.errors}"
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
    expiry_count = None
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
                    expiry_count = snapshot.counters.registrations_expired
                    emit(f"BBMD_FDT_EXPIRY_ARMED side={SIDE} expired={expiry_count}")

                if CHECK_EXPIRY_PATH.exists():
                    CHECK_EXPIRY_PATH.unlink()
                    snapshot = await control.snapshot()
                    if SIDE != "B" or snapshot.fdt:
                        raise AssertionError(f"side {SIDE} FDT did not expire: {snapshot.fdt!r}")
                    if (
                        expiry_count is None
                        or snapshot.counters.registrations_expired <= expiry_count
                    ):
                        raise AssertionError(
                            f"actor expiry counter did not advance: armed={expiry_count} "
                            f"current={snapshot.counters.registrations_expired}"
                        )
                    reported_expiry = True
                    emit(
                        f"BBMD_FDT_EXPIRED side={SIDE} "
                        f"expired={snapshot.counters.registrations_expired}"
                    )

                if REPORT_PATH.exists():
                    REPORT_PATH.unlink()
                    if SIDE == "B" and not reported_expiry:
                        raise AssertionError("side B report requested before FDT expiry was observable")
                    snapshot = await control.snapshot()
                    if SIDE == "A" and any(entry.ttl == 13 for entry in snapshot.fdt):
                        raise AssertionError("rejected TTL-13 probe mutated side A FDT")
                    validate_counters(control, snapshot)
                await asyncio.sleep(0.1)
    finally:
        await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
