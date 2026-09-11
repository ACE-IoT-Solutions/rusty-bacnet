"""Pinned-bacpypes3 roles for the W13 four-way semantic matrix."""

from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path

from bacpypes3.apdu import (
    ComplexAckPDU,
    ConfirmedCOVNotificationRequest,
    ConfirmedRequestPDU,
    ErrorPDU,
    ErrorRejectAbortNack,
    SimpleAckPDU,
)
from bacpypes3.appservice import ClientSSM
from bacpypes3.basetypes import PropertyIdentifier
from bacpypes3.ipv4.app import NormalApplication
from bacpypes3.local.analog import AnalogValueObject
from bacpypes3.object import DeviceObject
from bacpypes3.pdu import IPv4Address
from bacpypes3.primitivedata import ObjectIdentifier, Real


ARTIFACTS = Path(os.environ.get("W13_ARTIFACTS", "/artifacts"))
STACK = "bacpypes3"
OBJECT_COUNT = 220
DEVICE_INSTANCE = 4_171_300
DEVICE = ObjectIdentifier(("device", DEVICE_INSTANCE))
AV1 = ObjectIdentifier(("analog-value", 1))


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def oid_pair(value: ObjectIdentifier) -> list[int]:
    return [int(value[0]), int(value[1])]


async def wait_file(path: Path, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.02)


def build_application() -> tuple[NormalApplication, AnalogValueObject]:
    device = DeviceObject(
        objectIdentifier=("device", DEVICE_INSTANCE),
        objectName="W13 parity device",
        vendorIdentifier=999,
        vendorName="W13",
        modelName="four-way",
        maxApduLengthAccepted=1024,
        segmentationSupported="segmented-both",
        maxSegmentsAccepted=16,
        apduSegmentTimeout=1000,
    )
    app = NormalApplication(device, IPv4Address(os.environ["BACNET_LOCAL_ADDRESS"]))
    # bacpypes3 0.0.102 constructs its ASASAP before add_object() assigns
    # Application.device_object, so explicitly bind the segmentation profile
    # used by its ClientSSM/ServerSSM transaction state machines.
    app.asap.device_object = device
    first = None
    for instance in range(1, OBJECT_COUNT + 1):
        obj = AnalogValueObject(
            objectIdentifier=("analog-value", instance),
            objectName=f"W13 Analog Value {instance:03d}",
            presentValue=21.5 if instance == 1 else float(instance),
            description="four-way parity",
            statusFlags=[0, 0, 0, 0],
            eventState="normal",
            outOfService=False,
            units="degrees-celsius",
            covIncrement=0.5,
        )
        app.add_object(obj)
        if instance == 1:
            first = obj
    assert first is not None
    device.objectList = list(app.objectIdentifier)
    return app, first


async def run_server() -> None:
    app, first = build_application()
    print(
        f"W13_SERVER_READY stack={STACK} address={os.environ['BACNET_LOCAL_ADDRESS']}",
        flush=True,
    )
    commands = ARTIFACTS / "commands"
    commands.mkdir(parents=True, exist_ok=True)
    try:
        while True:
            for command in commands.glob("bacpypes3-*.trigger"):
                leg = command.name.removeprefix("bacpypes3-").removesuffix(".trigger")
                command.unlink()
                first.presentValue = Real(22.5)
                (commands / f"bacpypes3-{leg}.applied").touch()
                await wait_file(commands / f"bacpypes3-{leg}.done")
                (commands / f"bacpypes3-{leg}.done").unlink()
                first.presentValue = Real(21.5)
                (commands / f"bacpypes3-{leg}.reset").touch()
            await asyncio.sleep(0.02)
    finally:
        app.close()


def normalize_property(value: object) -> object:
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    try:
        return int(value)  # enumerations and unsigned values
    except (TypeError, ValueError):
        return str(value)


async def run_client() -> None:
    leg = os.environ["W13_LEG"]
    server_stack = os.environ["W13_SERVER_STACK"]
    target = IPv4Address(os.environ["BACNET_SERVER_ADDRESS"])
    device = DeviceObject(
        objectIdentifier=("device", 4_171_400 + int(os.environ["W13_LEG_ORDINAL"])),
        objectName=f"W13 {leg} client",
        vendorIdentifier=999,
        maxApduLengthAccepted=1024,
        segmentationSupported="segmented-both",
        maxSegmentsAccepted=16,
        apduSegmentTimeout=1000,
    )
    app = NormalApplication(device, IPv4Address(os.environ["BACNET_LOCAL_ADDRESS"]))
    app.asap.device_object = device
    request_sa_observations: list[bool] = []
    segmented_response_observations: list[bool] = []
    confirmed_cov_receipts: list[dict[str, int]] = []
    confirmed_cov_acks: list[dict[str, int]] = []
    original_get_segment = ClientSSM.get_segment
    original_confirmation = ClientSSM.confirmation
    original_cov_handler = app.do_ConfirmedCOVNotificationRequest
    original_response = app.response

    def observing_get_segment(transaction: ClientSSM, index: int):
        segment = original_get_segment(transaction, index)
        if isinstance(segment, ConfirmedRequestPDU):
            request_sa_observations.append(bool(segment.apduSA))
        return segment

    ClientSSM.get_segment = observing_get_segment

    async def observing_confirmation(transaction: ClientSSM, apdu):
        if isinstance(apdu, ComplexAckPDU):
            segmented_response_observations.append(bool(apdu.apduSeg))
        return await original_confirmation(transaction, apdu)

    ClientSSM.confirmation = observing_confirmation

    async def observing_cov_handler(apdu: ConfirmedCOVNotificationRequest) -> None:
        confirmed_cov_receipts.append(
            {
                "invoke_id": int(apdu.apduInvokeID),
                "process_id": int(apdu.subscriberProcessIdentifier),
            }
        )
        await original_cov_handler(apdu)

    async def observing_response(apdu) -> None:
        if isinstance(apdu, SimpleAckPDU) and int(apdu.apduService) == 1:
            confirmed_cov_acks.append(
                {"invoke_id": int(apdu.apduInvokeID), "service_choice": 1}
            )
        await original_response(apdu)

    app.do_ConfirmedCOVNotificationRequest = observing_cov_handler
    app.response = observing_response
    try:
        whole_error = None
        whole = []
        try:
            value = await app.read_property(target, DEVICE, "object-list")
            whole = [oid_pair(item) for item in value]
        except BaseException as error:
            whole_error = f"{type(error).__name__}: {error}"

        indexed_count = int(
            await app.read_property(target, DEVICE, "object-list", array_index=0)
        )
        indexed = []
        for index in range(1, indexed_count + 1):
            item = await app.read_property(
                target, DEVICE, "object-list", array_index=index
            )
            indexed.append(oid_pair(item))

        properties = []
        for prop, prop_id in (
            ("object-name", 77),
            ("present-value", 85),
            ("units", 117),
            ("description", 28),
        ):
            value = await app.read_property(target, AV1, prop)
            properties.append(
                {"property": prop_id, "array_index": None, "value": normalize_property(value)}
            )

        errors = []
        try:
            await app.read_property(
                target,
                ObjectIdentifier(("analog-value", 999_999)),
                "present-value",
            )
        except ErrorRejectAbortNack as error:
            if isinstance(error, ErrorPDU):
                errors.append(
                    {
                        "category": "bacnet_error",
                        "error_class": int(error.errorClass),
                        "error_code": int(error.errorCode),
                    }
                )
            else:
                errors.append({"category": type(error).__name__, "reason": int(error.reason)})

        commands = ARTIFACTS / "commands"
        async with app.change_of_value(
            target,
            AV1,
            subscriber_process_identifier=13_100 + int(os.environ["W13_LEG_ORDINAL"]),
            issue_confirmed_notifications=True,
            lifetime=30,
        ) as subscription:
            (commands / f"{server_stack}-{leg}.trigger").touch()
            cov_value = None
            async with asyncio.timeout(15):
                while cov_value != 22.5:
                    prop, value = await subscription.get_value()
                    if int(prop) == 85:
                        cov_value = float(value)
        (commands / f"{server_stack}-{leg}.done").touch()
        await wait_file(commands / f"{server_stack}-{leg}.reset")

        matching_receipts = [
            item
            for item in confirmed_cov_receipts
            if item["process_id"] == 13_100 + int(os.environ["W13_LEG_ORDINAL"])
        ]
        receipt_invoke_ids = {item["invoke_id"] for item in matching_receipts}
        matching_acks = [
            item for item in confirmed_cov_acks if item["invoke_id"] in receipt_invoke_ids
        ]

        inventory = sorted(item for item in indexed if item[0] == 2)
        result = {
            "schema_version": 1,
            "leg": leg,
            "client_stack": STACK,
            "server_stack": server_stack,
            "inventory": inventory,
            "topology": {"target": str(target), "route": None},
            "object_list": {
                "whole_count": len(whole) if whole else None,
                "whole_error": whole_error,
                "whole_values": whole,
                "indexed_count": indexed_count,
                "indexed_values": indexed,
            },
            "segmentation": {
                "client_profile": str(device.segmentationSupported),
                "outgoing_segmented_response_accepted": bool(request_sa_observations)
                and all(request_sa_observations),
                "confirmed_requests_observed": len(request_sa_observations),
                "segmented_response_observed": any(segmented_response_observations),
            },
            "properties": properties,
            "errors": errors,
            "cov": [
                {
                    "delivery": "confirmed",
                    "confirmed_request_observed": bool(matching_receipts),
                    "simple_ack_observed": bool(matching_acks),
                    "confirmed_request_count": len(matching_receipts),
                    "simple_ack_count": len(matching_acks),
                    "process_id": 13_100 + int(os.environ["W13_LEG_ORDINAL"]),
                    "value": cov_value,
                    "source_network": None,
                    "source_address_hex": None,
                }
            ],
        }
        atomic_json(ARTIFACTS / "legs" / f"{leg}.json", result)
        print(
            f"W13_LEG_PASS leg={leg} client={STACK} server={server_stack} "
            f"inventory={len(inventory)} indexed={indexed_count} cov=1 errors={len(errors)}",
            flush=True,
        )
    finally:
        ClientSSM.get_segment = original_get_segment
        ClientSSM.confirmation = original_confirmation
        app.do_ConfirmedCOVNotificationRequest = original_cov_handler
        app.response = original_response
        app.close()


async def main() -> None:
    role = sys.argv[1] if len(sys.argv) == 2 else ""
    if role == "server":
        await run_server()
    elif role == "client":
        await run_client()
    else:
        raise SystemExit("usage: w13_bacpypes_role.py {server|client}")


if __name__ == "__main__":
    asyncio.run(main())
