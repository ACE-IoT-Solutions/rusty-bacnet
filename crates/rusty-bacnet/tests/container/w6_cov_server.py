"""Installed-wheel BACnet server and deterministic local W6 COV writer."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

from rusty_bacnet import (
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


COORD = Path(os.environ.get("W6_COV_COORD", "/artifacts"))
COMMAND = COORD / "command"
STATE = COORD / "state"
PROGRESS = COORD / "progress"
OBJECTS = {
    "av": ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
    "bv": ObjectIdentifier(ObjectType.BINARY_VALUE, 2),
    "msv": ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 3),
    "csv": ObjectIdentifier(ObjectType.CHARACTERSTRING_VALUE, 4),
}


def publish(path: Path, value: str) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(value)
    temporary.replace(path)


async def wait_text(path: Path, expected: str, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while True:
            try:
                if path.read_text().strip() == expected:
                    return
            except FileNotFoundError:
                pass
            await asyncio.sleep(0.01)


async def wait_progress(expected: int) -> None:
    async with asyncio.timeout(5.0):
        while True:
            try:
                if int(PROGRESS.read_text().strip()) >= expected:
                    return
            except (FileNotFoundError, ValueError):
                pass
            await asyncio.sleep(0.002)


async def write(server: BACnetServer, kind: str, value: object) -> None:
    if kind == "av":
        encoded = PropertyValue.real(float(value))
    elif kind == "bv":
        encoded = PropertyValue.enumerated(int(value))
    elif kind == "msv":
        encoded = PropertyValue.unsigned(int(value))
    else:
        encoded = PropertyValue.character_string(str(value))
    await server.write_property_local(
        OBJECTS[kind], PropertyIdentifier.PRESENT_VALUE, encoded, priority=8
    )


async def reset(server: BACnetServer) -> None:
    await write(server, "av", 0.0)
    await write(server, "bv", 0)
    await write(server, "msv", 1)
    await write(server, "csv", "")


async def run_workload(server: BACnetServer) -> None:
    expected_notifications = 0
    last_values: dict[str, object] = {}
    for index in range(1, 251):
        values = {
            "av": index / 4.0,
            "bv": index % 2,
            "msv": (index % 3) + 1,
            "csv": f"value-{index:03d}",
        }
        for kind, value in values.items():
            await write(server, kind, value)
            last_values[kind] = value
            if kind != "av" or index % 4 == 0:
                expected_notifications += 1
                await wait_progress(expected_notifications)

    if expected_notifications != 812:
        raise AssertionError(f"server expected {expected_notifications} notifications")

    # Same-value writes are explicit no-spurious-notification guards.
    for kind, value in last_values.items():
        await write(server, kind, value)


async def main() -> None:
    COORD.mkdir(parents=True, exist_ok=True)
    server = BACnetServer(
        device_instance=int(os.environ.get("BACNET_DEVICE_INSTANCE", "4160600")),
        device_name="W6 COV interoperability fixture",
        interface=os.environ.get("BACNET_INTERFACE", "10.254.61.10"),
        port=int(os.environ.get("BACNET_PORT", "47808")),
        broadcast_address=os.environ.get("BACNET_BROADCAST", "10.254.61.255"),
    )
    server.add_analog_value(
        1,
        "W6 Analog Value",
        units=62,
        present_value=0.0,
        description="covIncrement filtering fixture",
        cov_increment=1.0,
    )
    server.add_binary_value(2, "W6 Binary Value", present_value=False)
    server.add_multistate_value(
        3,
        "W6 Multi-state Value",
        3,
        state_text=["one", "two", "three"],
        present_value=1,
    )
    server.add_character_string_value(4, "W6 Character String Value", present_value="")
    await server.start()
    print("W6_COV_SERVER_READY", flush=True)
    try:
        await wait_text(COMMAND, "reset")
        await reset(server)
        publish(STATE, "reset")

        await wait_text(COMMAND, "run")
        await run_workload(server)
        publish(STATE, "done")

        await wait_text(COMMAND, "expiry")
        await write(server, "bv", 0)
        await write(server, "bv", 1)
        publish(STATE, "expiry-done")
        await wait_text(COMMAND, "complete")
    finally:
        await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
