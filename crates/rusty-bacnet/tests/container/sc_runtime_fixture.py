import asyncio
import sys

import rusty_bacnet as bacnet


async def wait_for(predicate, message: str, timeout: float = 3.0) -> None:
    async with asyncio.timeout(timeout):
        while not await predicate():
            await asyncio.sleep(0.02)
    if not await predicate():
        raise AssertionError(message)


async def main(
    ca_cert: str,
    server_cert: str,
    server_key: str,
    client_cert: str,
    client_key: str,
    rogue_cert: str,
    rogue_key: str,
) -> None:
    primary = bacnet.ScHub(
        "127.0.0.1:0",
        server_cert,
        server_key,
        b"\x10\x00\x00\x00\x00\x01",
        ca_cert=ca_cert,
    )
    failover = bacnet.ScHub(
        "127.0.0.1:0",
        server_cert,
        server_key,
        b"\x20\x00\x00\x00\x00\x01",
        ca_cert=ca_cert,
    )
    await primary.start()
    await failover.start()
    primary_server = bacnet.BACnetServer(
        4200,
        "primary-mtls-device",
        transport="sc",
        sc_hub=await primary.url(),
        sc_vmac=b"\x02\x00\x00\x00\x00\x02",
        sc_ca_cert=ca_cert,
        sc_client_cert=client_cert,
        sc_client_key=client_key,
    )
    primary_server.add_analog_input(1, "Primary temperature", present_value=31.5)
    await primary_server.start()
    failover_server = bacnet.BACnetServer(
        4200,
        "failover-mtls-device",
        transport="sc",
        sc_hub=await failover.url(),
        sc_vmac=b"\x02\x00\x00\x00\x00\x03",
        sc_ca_cert=ca_cert,
        sc_client_cert=client_cert,
        sc_client_key=client_key,
    )
    failover_server.add_analog_input(1, "Failover temperature", present_value=32.5)
    await failover_server.start()
    runtime = None
    try:
        try:
            await bacnet.BACnetRuntime.start(
                [
                    bacnet.RuntimeScAttachment(
                        29,
                        "untrusted-client",
                        await primary.url(),
                        b"\x01\x02\x03\x04\x05\x05",
                        ca_cert=ca_cert,
                        client_cert=rogue_cert,
                        client_key=rogue_key,
                        reconnect_initial_delay_ms=20,
                        reconnect_max_delay_ms=20,
                        reconnect_max_retries=1,
                    )
                ]
            )
        except RuntimeError as error:
            assert getattr(error, "code", None) in {"TlsAuth", "ScDisconnected"}
        else:
            raise AssertionError("hub accepted an untrusted client certificate")

        runtime = await bacnet.BACnetRuntime.start(
            [
                bacnet.RuntimeScAttachment(
                    30,
                    "sc-runtime",
                    await primary.url(),
                    b"\x01\x02\x03\x04\x05\x06",
                    failover_hub=await failover.url(),
                    ca_cert=ca_cert,
                    client_cert=client_cert,
                    client_key=client_key,
                    reconnect_initial_delay_ms=50,
                    reconnect_max_delay_ms=50,
                    reconnect_max_retries=1,
                )
            ]
        )
        health = await runtime.health()
        assert health.attachment_states == ["Running"]
        assert health.attachment_error_codes == [None]
        topology = await runtime.topology(include_fdt=False)
        assert topology.error_codes == []
        assert list(topology.attachments[0].local_mac) == [1, 2, 3, 4, 5, 6]

        async def primary_device_visible() -> bool:
            discovery = await runtime.discover(timeout_ms=100)
            return any(
                device.attachment_id == 30
                and device.device_instance == 4200
                and list(device.path_mac) == [2, 0, 0, 0, 0, 2]
                for device in discovery.devices
            )

        await wait_for(primary_device_visible, "primary SC device was not discovered")
        initial = await runtime.read_batch(
            [bacnet.RuntimeRead(0, 30, 4200, 0, 1, 85, "real")]
        )
        assert initial[0].error_code is None
        assert round(initial[0].value.value, 1) == 31.5

        await primary.stop()
        await asyncio.sleep(0.2)

        async def failover_is_healthy() -> bool:
            current = await runtime.health()
            return current.attachment_states == ["Running"]

        await wait_for(failover_is_healthy, "runtime did not remain healthy on failover")

        async def failover_device_operates() -> bool:
            discovery = await runtime.discover(timeout_ms=100)
            failover_seen = any(
                device.attachment_id == 30
                and device.device_instance == 4200
                and list(device.path_mac) == [2, 0, 0, 0, 0, 3]
                for device in discovery.devices
            )
            if not failover_seen:
                return False
            result = await runtime.read_batch(
                [bacnet.RuntimeRead(0, 30, 4200, 0, 1, 85, "real")]
            )
            return (
                result[0].error_code is None
                and round(result[0].value.value, 1) == 32.5
            )

        await wait_for(
            failover_device_operates,
            "runtime did not complete BACnet work through the failover hub",
            timeout=6.0,
        )

        await failover.stop()

        async def both_hubs_lost() -> bool:
            current = await runtime.health()
            return current.attachment_error_codes == ["ScDisconnected"]

        await wait_for(both_hubs_lost, "runtime did not expose BACnet/SC hub loss")
    finally:
        if runtime is not None:
            await runtime.stop()
        await primary.stop()
        await failover.stop()
        await primary_server.stop()
        await failover_server.stop()


asyncio.run(main(*sys.argv[1:8]))
