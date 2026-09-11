"""Installed-wheel Rust managed foreign-device lifecycle acceptance role."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

from rusty_bacnet import BACnetRuntime, RuntimeAttachment


ARTIFACT_PATH = Path(os.environ.get("W2_ARTIFACT_PATH", "/tmp/w2-foreign-transitions.json"))
SHUTDOWN_PATH = Path("/tmp/w2-shutdown")
DEVICE_INSTANCE = int(os.environ.get("BACNET_DEVICE_INSTANCE", "4190202"))
EXPECTED_STATES = [
    "pending",
    "registered",
    "expired",
    "registered",
    "rejected",
    "expired",
    "registered",
]


def emit(message: str) -> None:
    print(message, flush=True)


def marker_for(index: int) -> str:
    return (
        "W2_FD_PENDING",
        "W2_FD_REGISTERED",
        "W2_FD_EXPIRED reason=bbmd_outage",
        "W2_FD_RECOVERED reason=bbmd_restart",
        "W2_FD_REJECTED",
        "W2_FD_EXPIRED reason=rejection",
        "W2_FD_RECOVERED reason=rejection_clear",
    )[index]


async def main() -> None:
    interface = os.environ["BACNET_INTERFACE"]
    port = int(os.environ.get("BACNET_PORT", "47810"))
    broadcast = os.environ["BACNET_BROADCAST"]
    bbmd_address = os.environ["BACNET_BBMD_ADDRESS"]
    ttl = int(os.environ.get("BACNET_FOREIGN_DEVICE_TTL", "4"))
    attachment = RuntimeAttachment(
        2,
        "w2-foreign",
        interface,
        port,
        broadcast,
        bbmd_address,
        ttl,
    )
    runtime = await BACnetRuntime.start([attachment], event_capacity=64, max_event_batch=32)
    transitions: list[dict[str, object]] = []
    next_state = 0
    saw_broadcast = False
    stopped = False
    try:
        async with asyncio.timeout(100):
            while True:
                health = await runtime.health()
                if health.attachment_states != ["Running"]:
                    raise AssertionError(
                        f"foreign attachment did not remain Running: {health.attachment_states!r}"
                    )
                if len(health.foreign_device_statuses) != 1:
                    raise AssertionError("foreign-device status cardinality mismatch")
                status = health.foreign_device_statuses[0]
                if status is None:
                    raise AssertionError("foreign attachment omitted registration telemetry")
                expected = EXPECTED_STATES[next_state] if next_state < len(EXPECTED_STATES) else None
                if status.state == expected:
                    degraded = status.state in ("rejected", "expired")
                    error = health.attachment_error_codes[0]
                    if degraded and error != "ForeignDeviceRegistrationFailed":
                        raise AssertionError(
                            f"state {status.state} did not project registration error: {error!r}"
                        )
                    if not degraded and error is not None:
                        raise AssertionError(
                            f"healthy state {status.state} retained attachment error: {error!r}"
                        )
                    if status.state == "rejected" and status.last_result != 0x0030:
                        raise AssertionError(
                            f"rejected state had wrong BVLC result: {status.last_result!r}"
                        )
                    transitions.append(
                        {
                            "sequence": next_state + 1,
                            "state": status.state,
                            "last_result": status.last_result,
                            "error": error,
                        }
                    )
                    emit(
                        f"{marker_for(next_state)} last_result={status.last_result} "
                        f"error={error}"
                    )
                    next_state += 1

                batch = await runtime.next_events(max_items=32, wait_ms=0)
                for event in batch.events:
                    if event.kind != "i_am_observation" or event.device_instance != DEVICE_INSTANCE:
                        continue
                    if event.bvlc_function != 0x04:
                        raise AssertionError(
                            f"BBMD broadcast was not received as Forwarded-NPDU: {event.bvlc_function!r}"
                        )
                    saw_broadcast = True
                    emit(
                        f"W2_FD_BROADCAST_RECEIVED instance={event.device_instance} "
                        f"bvlc_function=0x{event.bvlc_function:02x}"
                    )

                if SHUTDOWN_PATH.exists():
                    if next_state != len(EXPECTED_STATES) or not saw_broadcast:
                        raise AssertionError(
                            f"shutdown requested before acceptance completed: "
                            f"states={next_state}/{len(EXPECTED_STATES)} broadcast={saw_broadcast}"
                        )
                    SHUTDOWN_PATH.unlink()
                    await runtime.stop()
                    stopped = True
                    emit("W2_FD_SHUTDOWN_COMPLETE")
                    break
                await asyncio.sleep(0.05)
    finally:
        if not stopped:
            await runtime.stop()
        ARTIFACT_PATH.parent.mkdir(parents=True, exist_ok=True)
        ARTIFACT_PATH.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "expected_states": EXPECTED_STATES,
                    "transitions": transitions,
                    "broadcast_received": saw_broadcast,
                    "shutdown_complete": stopped,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
    emit("W2_FOREIGN_LIFECYCLE_ROLE_PASS")


if __name__ == "__main__":
    asyncio.run(main())
