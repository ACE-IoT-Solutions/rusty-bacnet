"""Pinned-bacpypes3 whole-APDU subscriber for deterministic W6 COV acceptance."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
from typing import Any

from bacpypes3.apdu import (
    ConfirmedCOVNotificationRequest,
    SimpleAckPDU,
    SubscribeCOVRequest,
    UnconfirmedCOVNotificationRequest,
)
from bacpypes3.basetypes import BinaryPV
from bacpypes3.ipv4.app import NormalApplication
from bacpypes3.object import DeviceObject
from bacpypes3.pdu import IPv4Address
from bacpypes3.primitivedata import CharacterString, ObjectIdentifier, PropertyIdentifier, Real, Unsigned


SERVER = IPv4Address(os.environ.get("BACNET_SERVER_ADDRESS", "10.254.61.10:47808"))
LOCAL = IPv4Address(os.environ.get("BACNET_LOCAL_ADDRESS", "10.254.61.20/24:47808"))
COORD = Path(os.environ.get("W6_COV_COORD", "/artifacts"))
COMMAND = COORD / "command"
STATE = COORD / "state"
PROGRESS = COORD / "progress"
ARTIFACT = COORD / "w6-cov-results.json"
OBJECTS = {
    "av": ObjectIdentifier("analog-value,1"),
    "bv": ObjectIdentifier("binary-value,2"),
    "msv": ObjectIdentifier("multi-state-value,3"),
    "csv": ObjectIdentifier("characterstring-value,4"),
}
PROCESS = {
    ("confirmed", "av"): 6000,
    ("confirmed", "bv"): 6001,
    ("confirmed", "msv"): 6002,
    ("confirmed", "csv"): 6003,
    ("unconfirmed", "av"): 6100,
    ("unconfirmed", "bv"): 6101,
    ("unconfirmed", "msv"): 6102,
    ("unconfirmed", "csv"): 6103,
}
PROCESS_LOOKUP = {value: key for key, value in PROCESS.items()}


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


class CovProbeApplication(NormalApplication):
    def __init__(self, device_object: DeviceObject, local_address: IPv4Address) -> None:
        super().__init__(device_object, local_address)
        self.notifications: asyncio.Queue[tuple[str, Any]] = asyncio.Queue()

    async def do_ConfirmedCOVNotificationRequest(
        self, apdu: ConfirmedCOVNotificationRequest
    ) -> None:
        await self.notifications.put(("confirmed", apdu))
        await self.response(SimpleAckPDU(context=apdu))

    async def do_UnconfirmedCOVNotificationRequest(
        self, apdu: UnconfirmedCOVNotificationRequest
    ) -> None:
        await self.notifications.put(("unconfirmed", apdu))


def normalize(kind: str, value: Any) -> float | int | str:
    if kind == "av":
        return round(float(value), 6)
    if kind in ("bv", "msv"):
        return int(value)
    return str(value)


def decode_notification(mode: str, apdu: Any) -> tuple[str, float | int | str]:
    process = int(apdu.subscriberProcessIdentifier)
    expected_identity = PROCESS_LOOKUP.get(process)
    if expected_identity is None:
        if process == 6200:
            expected_identity = ("expiry", "bv")
        else:
            raise AssertionError(f"unknown subscriber process {process}")
    expected_mode, kind = expected_identity
    if expected_mode != "expiry" and mode != expected_mode:
        raise AssertionError(f"process {process}: {mode} delivery != {expected_mode}")
    if apdu.monitoredObjectIdentifier != OBJECTS[kind]:
        raise AssertionError(
            f"process {process}: object {apdu.monitoredObjectIdentifier!r} != {OBJECTS[kind]!r}"
        )
    values = apdu.listOfValues
    if len(values) != 2:
        raise AssertionError(f"process {process}: {len(values)} COV values != 2")
    if values[0].propertyIdentifier != PropertyIdentifier.presentValue:
        raise AssertionError(f"process {process}: first property is not present-value")
    if values[1].propertyIdentifier != PropertyIdentifier.statusFlags:
        raise AssertionError(f"process {process}: second property is not status-flags")
    value_type = {
        "av": Real,
        "bv": BinaryPV,
        "msv": Unsigned,
        "csv": CharacterString,
    }[kind]
    value = values[0].value.cast_out(value_type)
    return kind, normalize(kind, value)


async def receive(app: CovProbeApplication) -> tuple[str, str, float | int | str]:
    async with asyncio.timeout(5):
        mode, apdu = await app.notifications.get()
    kind, value = decode_notification(mode, apdu)
    return mode, kind, value


async def subscribe(
    app: CovProbeApplication,
    process: int,
    oid: ObjectIdentifier,
    confirmed: bool,
    lifetime: int,
) -> None:
    response = await app.request(
        SubscribeCOVRequest(
            subscriberProcessIdentifier=process,
            monitoredObjectIdentifier=oid,
            issueConfirmedNotifications=confirmed,
            lifetime=lifetime,
            destination=SERVER,
        )
    )
    if not isinstance(response, SimpleAckPDU):
        raise AssertionError(f"SubscribeCOV process {process} response: {response!r}")


async def cancel(app: CovProbeApplication, process: int, oid: ObjectIdentifier) -> None:
    response = await app.request(
        SubscribeCOVRequest(
            subscriberProcessIdentifier=process,
            monitoredObjectIdentifier=oid,
            destination=SERVER,
        )
    )
    if not isinstance(response, SimpleAckPDU):
        raise AssertionError(f"cancel process {process} response: {response!r}")


async def assert_quiet(app: CovProbeApplication, reason: str) -> None:
    await asyncio.sleep(0.4)
    if not app.notifications.empty():
        mode, apdu = app.notifications.get_nowait()
        raise AssertionError(
            f"{reason}: spurious {mode} notification process={apdu.subscriberProcessIdentifier}"
        )


async def main() -> None:
    COORD.mkdir(parents=True, exist_ok=True)
    device = DeviceObject(
        objectIdentifier=("device", 4160601),
        objectName="W6 bacpypes3 subscriber",
        vendorIdentifier=999,
    )
    app = CovProbeApplication(device, LOCAL)
    try:
        publish(COMMAND, "reset")
        await wait_text(STATE, "reset")

        initial_expected = {"av": 0.0, "bv": 0, "msv": 1, "csv": ""}
        for mode, confirmed in (("confirmed", True), ("unconfirmed", False)):
            for kind, oid in OBJECTS.items():
                await subscribe(app, PROCESS[(mode, kind)], oid, confirmed, 0)
                actual_mode, actual_kind, value = await receive(app)
                if (actual_mode, actual_kind, value) != (mode, kind, initial_expected[kind]):
                    raise AssertionError(
                        f"initial {(actual_mode, actual_kind, value)!r} != "
                        f"{(mode, kind, initial_expected[kind])!r}"
                    )
        await assert_quiet(app, "after eight initial notifications")

        expected_by_mode = {"confirmed": [], "unconfirmed": []}
        observed_by_mode = {"confirmed": [], "unconfirmed": []}
        counts = {
            "confirmed": {kind: 0 for kind in OBJECTS},
            "unconfirmed": {kind: 0 for kind in OBJECTS},
        }
        publish(PROGRESS, "0")
        publish(COMMAND, "run")
        logical_notification = 0
        update_number = 0
        for index in range(1, 251):
            values = {
                "av": index / 4.0,
                "bv": index % 2,
                "msv": (index % 3) + 1,
                "csv": f"value-{index:03d}",
            }
            for kind, value in values.items():
                update_number += 1
                if kind == "av" and index % 4 != 0:
                    continue
                logical_notification += 1
                event = {
                    "update": update_number,
                    "object": kind,
                    "value": normalize(kind, value),
                }
                pair = [await receive(app), await receive(app)]
                if {item[0] for item in pair} != {"confirmed", "unconfirmed"}:
                    raise AssertionError(f"update {update_number}: delivery pair {pair!r}")
                for mode, actual_kind, actual_value in pair:
                    observed = {
                        "update": update_number,
                        "object": actual_kind,
                        "value": actual_value,
                    }
                    expected_by_mode[mode].append(event)
                    observed_by_mode[mode].append(observed)
                    if observed != event:
                        raise AssertionError(f"{mode} COV {observed!r} != {event!r}")
                    counts[mode][kind] += 1
                publish(PROGRESS, str(logical_notification))

        await wait_text(STATE, "done")
        if update_number != 1000 or logical_notification != 812:
            raise AssertionError(
                f"workload updates={update_number}, logical notifications={logical_notification}"
            )
        expected_counts = {"av": 62, "bv": 250, "msv": 250, "csv": 250}
        for mode in ("confirmed", "unconfirmed"):
            if counts[mode] != expected_counts:
                raise AssertionError(f"{mode} counts mismatch: {counts[mode]!r}")
            if observed_by_mode[mode] != expected_by_mode[mode]:
                raise AssertionError(f"{mode} sequence mismatch")
        await assert_quiet(app, "after workload and same-value writes")

        for mode in ("confirmed", "unconfirmed"):
            for kind, oid in OBJECTS.items():
                await cancel(app, PROCESS[(mode, kind)], oid)
            print(
                f"W6_COV_MODE_PASS mode={mode} updates=1000 expected=812 observed=812 "
                "av=62 bv=250 msv=250 csv=250",
                flush=True,
            )

        await subscribe(app, 6200, OBJECTS["bv"], False, 2)
        mode, kind, value = await receive(app)
        if (mode, kind, value) != ("unconfirmed", "bv", 0):
            raise AssertionError(f"expiry initial {(mode, kind, value)!r}")
        await asyncio.sleep(2.35)
        publish(COMMAND, "expiry")
        await wait_text(STATE, "expiry-done")
        await assert_quiet(app, "after finite subscription expiry")
        await cancel(app, 6200, OBJECTS["bv"])
        print("W6_COV_EXPIRY_PASS lifetime=2 post_expiry_notifications=0", flush=True)

        results = {
            "bacpypes3_version": "0.0.102",
            "updates": 1000,
            "modes": {
                mode: {
                    "counts": counts[mode],
                    "expected": expected_by_mode[mode],
                    "observed": observed_by_mode[mode],
                }
                for mode in ("confirmed", "unconfirmed")
            },
            "expiry": {"lifetime_seconds": 2, "post_expiry_notifications": 0},
        }
        ARTIFACT.write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
        publish(COMMAND, "complete")
        print(f"W6_COV_ARTIFACT path={ARTIFACT}", flush=True)
        print("W6_COV_ACCEPTANCE_PASS", flush=True)
    finally:
        app.close()


if __name__ == "__main__":
    asyncio.run(main())
