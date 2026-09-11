"""Installed-wheel Rust roles for the W13 four-way semantic matrix."""

from __future__ import annotations

import asyncio
import json
import os
import socket
import sys
from pathlib import Path

from rusty_bacnet import (
    BACnetClient,
    BACnetRuntime,
    BACnetServer,
    BacnetProtocolError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    RuntimeAttachment,
    RuntimePersistedDevice,
    RuntimeRead,
)


ARTIFACTS = Path(os.environ.get("W13_ARTIFACTS", "/artifacts"))
STACK = "rusty"
OBJECT_COUNT = 220
DEVICE_INSTANCE = 4_171_300
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
DEVICE = ObjectIdentifier(ObjectType.DEVICE, DEVICE_INSTANCE)


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def oid_pair(value: ObjectIdentifier) -> list[int]:
    return [value.object_type.to_raw(), value.instance]


async def wait_file(path: Path, timeout: float = 30.0) -> None:
    async with asyncio.timeout(timeout):
        while not path.exists():
            await asyncio.sleep(0.02)


async def run_server() -> None:
    interface = os.environ["BACNET_INTERFACE"]
    port = int(os.environ["BACNET_PORT"])
    server = BACnetServer(
        device_instance=DEVICE_INSTANCE,
        device_name="W13 parity device",
        vendor_name="W13",
        vendor_identifier=999,
        model_name="four-way",
        interface=interface,
        port=port,
        broadcast_address=os.environ["BACNET_BROADCAST"],
    )
    for instance in range(1, OBJECT_COUNT + 1):
        server.add_analog_value(
            instance,
            f"W13 Analog Value {instance:03d}",
            units=62,
            present_value=21.5 if instance == 1 else float(instance),
            description="four-way parity",
            cov_increment=0.5,
        )
    await server.start()
    print(f"W13_SERVER_READY stack={STACK} address={await server.local_address()}", flush=True)
    commands = ARTIFACTS / "commands"
    commands.mkdir(parents=True, exist_ok=True)
    try:
        while True:
            for command in commands.glob("rusty-*.trigger"):
                leg = command.name.removeprefix("rusty-").removesuffix(".trigger")
                command.unlink()
                await server.write_property_local(
                    AV1, PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(22.5), priority=8
                )
                (commands / f"rusty-{leg}.applied").touch()
                await wait_file(commands / f"rusty-{leg}.done")
                (commands / f"rusty-{leg}.done").unlink()
                await server.write_property_local(
                    AV1, PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(21.5), priority=8
                )
                (commands / f"rusty-{leg}.reset").touch()
            await asyncio.sleep(0.02)
    finally:
        await server.stop()


async def run_client() -> None:
    leg = os.environ["W13_LEG"]
    server_stack = os.environ["W13_SERVER_STACK"]
    target = os.environ["BACNET_SERVER_ADDRESS"]
    client = BACnetClient(
        interface=os.environ["BACNET_INTERFACE"],
        port=int(os.environ["BACNET_PORT"]),
        broadcast_address=os.environ["BACNET_BROADCAST"],
        apdu_timeout_ms=3_000,
    )
    await client.__aenter__()
    try:
        whole_error = None
        whole = []
        try:
            value = await client.read_property(target, DEVICE, PropertyIdentifier.OBJECT_LIST)
            whole = [oid_pair(item) for item in value.value]
        except Exception as error:
            whole_error = f"{type(error).__name__}: {error}"

        rpm_whole = await client.read_property_multiple(
            target,
            [(DEVICE, [(PropertyIdentifier.OBJECT_LIST, None)])],
        )
        rpm_whole = [oid_pair(item) for item in rpm_whole[0]["results"][0]["value"].value]

        await client.add_device(DEVICE_INSTANCE, target)
        auto_whole_value = await client.read_property_from_device(
            DEVICE_INSTANCE, DEVICE, PropertyIdentifier.OBJECT_LIST
        )
        auto_whole = [oid_pair(item) for item in auto_whole_value.value]

        batch_rp = await client.read_property_from_devices(
            [(DEVICE_INSTANCE, DEVICE, PropertyIdentifier.OBJECT_LIST, None)]
        )
        batch_rp_whole = [oid_pair(item) for item in batch_rp[0]["value"].value]

        batch_rpm = await client.read_property_multiple_from_devices(
            [
                (
                    DEVICE_INSTANCE,
                    [(DEVICE, [(PropertyIdentifier.OBJECT_LIST, None)])],
                )
            ]
        )
        batch_rpm_value = batch_rpm[0]["results"][0]["results"][0]["value"]
        batch_rpm_whole = [oid_pair(item) for item in batch_rpm_value.value]

        server_host, server_port = target.rsplit(":", 1)
        runtime_port = int(os.environ["BACNET_PORT"]) + 100
        runtime = await BACnetRuntime.start(
            [
                RuntimeAttachment(
                    1,
                    f"W13 {leg} runtime",
                    os.environ["BACNET_INTERFACE"],
                    runtime_port,
                    os.environ["BACNET_BROADCAST"],
                )
            ]
        )
        try:
            server_mac = list(
                socket.inet_aton(server_host) + int(server_port).to_bytes(2, "big")
            )
            restored = await runtime.restore_devices(
                [RuntimePersistedDevice(1, DEVICE_INSTANCE, server_mac)]
            )
            if restored.added != 1:
                raise RuntimeError(f"runtime restore did not add device: {restored.added}")
            runtime_result = await runtime.read_batch(
                [
                    RuntimeRead(
                        0,
                        1,
                        DEVICE_INSTANCE,
                        ObjectType.DEVICE.to_raw(),
                        DEVICE_INSTANCE,
                        PropertyIdentifier.OBJECT_LIST.to_raw(),
                        "object_identifier",
                    )
                ]
            )
            if runtime_result[0].error_code is not None:
                raise RuntimeError(f"runtime whole read failed: {runtime_result[0].error_code}")
            runtime_whole = [oid_pair(item) for item in runtime_result[0].value.value]
        finally:
            await runtime.stop()

        count_value = await client.read_property(
            target, DEVICE, PropertyIdentifier.OBJECT_LIST, array_index=0
        )
        indexed_count = int(count_value.value)
        indexed = []
        for index in range(1, indexed_count + 1):
            item = await client.read_property(
                target, DEVICE, PropertyIdentifier.OBJECT_LIST, array_index=index
            )
            indexed.append(oid_pair(item.value))

        rpm = await client.read_property_multiple(
            target,
            [
                (
                    AV1,
                    [
                        (PropertyIdentifier.OBJECT_NAME, None),
                        (PropertyIdentifier.PRESENT_VALUE, None),
                        (PropertyIdentifier.UNITS, None),
                        (PropertyIdentifier.DESCRIPTION, None),
                    ],
                )
            ],
        )
        properties = []
        for item in rpm[0]["results"]:
            value = item["value"].value if item["value"] is not None else None
            properties.append(
                {
                    "property": item["property_id"].to_raw(),
                    "array_index": item["array_index"],
                    "value": value,
                }
            )

        errors = []
        try:
            await client.read_property(
                target,
                ObjectIdentifier(ObjectType.ANALOG_VALUE, 999_999),
                PropertyIdentifier.PRESENT_VALUE,
            )
        except BacnetProtocolError as error:
            errors.append(
                {"category": "bacnet_error", "error_class": error.error_class, "error_code": error.error_code}
            )

        notifications = await client.cov_notifications()
        process_id = 13_100 + int(os.environ["W13_LEG_ORDINAL"])
        await client.subscribe_cov(
            target,
            subscriber_process_identifier=process_id,
            monitored_object_identifier=AV1,
            confirmed=True,
            lifetime=30,
        )
        commands = ARTIFACTS / "commands"
        trigger = commands / f"{server_stack}-{leg}.trigger"
        trigger.touch()
        cov = None
        async with asyncio.timeout(15):
            async for notification in notifications:
                for changed in notification.values:
                    if changed["property_id"] != PropertyIdentifier.PRESENT_VALUE:
                        continue
                    decoded = changed["value"]
                    if decoded is not None and float(decoded.value) == 22.5:
                        cov = {
                            "delivery": notification.delivery,
                            "process_id": notification.subscriber_process_identifier,
                            "value": float(decoded.value),
                            "source_network": notification.source_network,
                            "source_address_hex": (
                                notification.source_address.hex()
                                if notification.source_address is not None
                                else None
                            ),
                        }
                        break
                if cov is not None:
                    break
        await client.unsubscribe_cov(
            target,
            subscriber_process_identifier=process_id,
            monitored_object_identifier=AV1,
        )
        (commands / f"{server_stack}-{leg}.done").touch()
        await wait_file(commands / f"{server_stack}-{leg}.reset")

        inventory = sorted(
            item for item in indexed if item[0] == ObjectType.ANALOG_VALUE.to_raw()
        )
        result = {
            "schema_version": 1,
            "leg": leg,
            "client_stack": STACK,
            "server_stack": server_stack,
            "inventory": inventory,
            "topology": {"target": target, "route": None},
            "object_list": {
                "whole_count": len(whole) if whole else None,
                "whole_error": whole_error,
                "whole_values": whole,
                "rpm_whole_count": len(rpm_whole),
                "rpm_whole_values": rpm_whole,
                "auto_whole_count": len(auto_whole),
                "auto_whole_values": auto_whole,
                "batch_rp_whole_count": len(batch_rp_whole),
                "batch_rp_whole_values": batch_rp_whole,
                "batch_rpm_whole_count": len(batch_rpm_whole),
                "batch_rpm_whole_values": batch_rpm_whole,
                "runtime_whole_count": len(runtime_whole),
                "runtime_whole_values": runtime_whole,
                "indexed_count": indexed_count,
                "indexed_values": indexed,
            },
            "properties": properties,
            "errors": errors,
            "cov": [cov],
        }
        atomic_json(ARTIFACTS / "legs" / f"{leg}.json", result)
        print(
            f"W13_LEG_PASS leg={leg} client={STACK} server={server_stack} "
            f"inventory={len(inventory)} indexed={indexed_count} cov=1 errors={len(errors)}",
            flush=True,
        )
    finally:
        await client.stop()


async def main() -> None:
    role = sys.argv[1] if len(sys.argv) == 2 else ""
    if role == "server":
        await run_server()
    elif role == "client":
        await run_client()
    else:
        raise SystemExit("usage: w13_rust_role.py {server|client}")


if __name__ == "__main__":
    asyncio.run(main())
