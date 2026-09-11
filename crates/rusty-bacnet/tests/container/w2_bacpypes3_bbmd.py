"""Pinned-bacpypes3 BBMD and broadcaster for the W2 lifecycle fixture."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
import socket
import sys
import time

from bacpypes3.ipv4.bvll import Result
from bacpypes3.ipv4.link import BBMDLinkLayer
from bacpypes3.pdu import IPv4Address


REJECT_PATH = Path("/tmp/w2-reject")
ALLOW_PATH = Path("/tmp/w2-allow")
REPORT_PATH = Path("/tmp/w2-report")
CLEANUP_PATH = Path("/tmp/w2-arm-shutdown-cleanup")
STOP_PATH = Path("/tmp/w2-stop-bbmd")
START_PATH = Path("/tmp/w2-start-bbmd")
ARTIFACT_PATH = Path(os.environ.get("W2_ARTIFACT_PATH", "/tmp/w2-bbmd-events.jsonl"))
EXPECTED_TTL = int(os.environ.get("BACNET_FOREIGN_DEVICE_TTL", "4"))


def emit(message: str) -> None:
    print(message, flush=True)


def append_event(kind: str, **fields: object) -> None:
    ARTIFACT_PATH.parent.mkdir(parents=True, exist_ok=True)
    record = {"kind": kind, **fields}
    with ARTIFACT_PATH.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(record, sort_keys=True) + "\n")


class LifecycleBBMD(BBMDLinkLayer):
    """BBMD with observable registration results and a deterministic reject gate."""

    def __init__(self, address: IPv4Address) -> None:
        super().__init__(address)
        self.registration_count = 0
        self.success_times: list[float] = []
        self.half_ttl_reported = False
        self.unregister_count = 0
        self.cleanup_registration_count: int | None = None

    async def request(self, lpdu: object) -> None:
        if isinstance(lpdu, Result):
            frame = bytes(lpdu.encode().pduData)
            source = f"{self.bbmdAddress.addrTuple[0]}:{self.bbmdAddress.addrTuple[1]}"
            append_event(
                "result_response",
                source=source,
                destination=str(lpdu.pduDestination),
                function=0x00,
                result=lpdu.bvlciResultCode,
                frame_hex=frame.hex(),
                unicast=True,
            )
            emit(
                f"W2_BBMD_RESULT source={source} "
                f"destination={lpdu.pduDestination} function=0x00 "
                f"result=0x{lpdu.bvlciResultCode:04x} frame={frame.hex()} unicast=true"
            )
        await super().request(lpdu)

    def register_foreign_device(self, addr: IPv4Address, ttl: int) -> int:
        self.registration_count += 1
        now = time.monotonic()
        rejected = REJECT_PATH.exists() and not ALLOW_PATH.exists()
        result = 0x0030 if rejected else super().register_foreign_device(addr, ttl)
        append_event(
            "register",
            sequence=self.registration_count,
            source=str(addr),
            ttl=ttl,
            result=result,
            monotonic=round(now, 6),
        )
        emit(
            f"W2_BBMD_REGISTER sequence={self.registration_count} source={addr} "
            f"ttl={ttl} result=0x{result:04x}"
        )
        if result == 0:
            self.success_times.append(now)
            if len(self.success_times) >= 2 and not self.half_ttl_reported:
                interval = self.success_times[-1] - self.success_times[-2]
                expected = EXPECTED_TTL / 2.0
                if not expected * 0.65 <= interval <= expected * 1.65:
                    raise AssertionError(
                        f"renewal interval {interval:.3f}s was not near TTL/2 ({expected:.3f}s)"
                    )
                self.half_ttl_reported = True
                append_event("ttl_half_renewal", interval_seconds=round(interval, 3))
                emit(
                    f"W2_BBMD_TTL_HALF_RENEWAL interval={interval:.3f} "
                    f"ttl={EXPECTED_TTL}"
                )
        else:
            # A rejection breaks the consecutive-success series.  Recovery
            # starts a new series rather than comparing across rejected cycles.
            self.success_times.clear()
        return result

    def delete_foreign_device_table_entry(self, addr: IPv4Address) -> int:
        result = super().delete_foreign_device_table_entry(addr)
        self.unregister_count += 1
        append_event("unregister", source=str(addr), result=result)
        emit(f"W2_BBMD_UNREGISTERED source={addr} result=0x{result:04x}")
        return result


async def run_bbmd() -> None:
    address = IPv4Address(os.environ.get("BACNET_BBMD_ADDRESS", "10.252.10.10/24:47808"))
    ALLOW_PATH.unlink(missing_ok=True)
    REJECT_PATH.unlink(missing_ok=True)
    STOP_PATH.unlink(missing_ok=True)
    START_PATH.unlink(missing_ok=True)
    bbmd: LifecycleBBMD | None = LifecycleBBMD(address)
    emit(f"W2_BBMD_READY address={address}")

    def close_bbmd(current: LifecycleBBMD) -> None:
        current._fdt_clock_handle.cancel()
        current.close()

    try:
        async with asyncio.timeout(120):
            while True:
                if STOP_PATH.exists():
                    STOP_PATH.unlink()
                    if bbmd is not None:
                        close_bbmd(bbmd)
                        bbmd = None
                        append_event("service_stopped", generation=1)
                        emit("W2_BBMD_STOPPED generation=1")
                if START_PATH.exists():
                    START_PATH.unlink()
                    if bbmd is None:
                        bbmd = LifecycleBBMD(address)
                        append_event("service_restarted", generation=2)
                        emit(f"W2_BBMD_RESTARTED generation=2 address={address}")
                if ALLOW_PATH.exists():
                    ALLOW_PATH.unlink()
                    REJECT_PATH.unlink(missing_ok=True)
                    append_event("mode", value="allow")
                    emit("W2_BBMD_MODE mode=allow")
                if REPORT_PATH.exists() and bbmd is not None:
                    REPORT_PATH.unlink()
                    append_event(
                        "summary",
                        registrations=bbmd.registration_count,
                        unregisters=bbmd.unregister_count,
                        fdt_entries=len(bbmd.bbmdFDT),
                    )
                    emit(
                        f"W2_BBMD_SUMMARY registrations={bbmd.registration_count} "
                        f"unregisters={bbmd.unregister_count} fdt_entries={len(bbmd.bbmdFDT)}"
                    )
                if (
                    CLEANUP_PATH.exists()
                    and bbmd is not None
                    and bbmd.cleanup_registration_count is None
                ):
                    CLEANUP_PATH.unlink()
                    bbmd.cleanup_registration_count = bbmd.registration_count
                    append_event(
                        "shutdown_cleanup_armed",
                        registrations=bbmd.cleanup_registration_count,
                        fdt_entries=len(bbmd.bbmdFDT),
                    )
                    emit(
                        f"W2_BBMD_SHUTDOWN_CLEANUP_ARMED "
                        f"registrations={bbmd.cleanup_registration_count}"
                    )
                if (
                    bbmd is not None
                    and bbmd.cleanup_registration_count is not None
                    and not bbmd.bbmdFDT
                ):
                    if bbmd.registration_count != bbmd.cleanup_registration_count:
                        raise AssertionError(
                            "foreign device renewed after runtime shutdown: "
                            f"armed={bbmd.cleanup_registration_count} "
                            f"current={bbmd.registration_count}"
                        )
                    append_event(
                        "shutdown_cleanup_complete",
                        registrations=bbmd.registration_count,
                        fdt_entries=0,
                    )
                    emit(
                        f"W2_BBMD_FDT_CLEANUP_COMPLETE registrations_stable="
                        f"{bbmd.registration_count} fdt_entries=0"
                    )
                    bbmd.cleanup_registration_count = None
                await asyncio.sleep(0.05)
    finally:
        if bbmd is not None:
            close_bbmd(bbmd)


def run_broadcaster() -> None:
    interface = os.environ["BACNET_INTERFACE"]
    broadcast = os.environ["BACNET_BROADCAST"]
    port = int(os.environ.get("BACNET_PORT", "47808"))
    instance = int(os.environ.get("BACNET_DEVICE_INSTANCE", "4190202"))
    device_oid = (8 << 22) | instance
    service_request = (
        b"\xc4"
        + device_oid.to_bytes(4, "big")
        + b"\x22\x05\xc4"
        + b"\x91\x03"
        + b"\x21\x0f"
    )
    npdu = b"\x01\x00\x10\x00" + service_request
    frame = b"\x81\x0b" + (len(npdu) + 4).to_bytes(2, "big") + npdu
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sock.bind((interface, port))
        sock.sendto(frame, (broadcast, port))
    emit(f"W2_BROADCAST_SENT instance={instance} origin={interface}:{port}")


async def main() -> None:
    role = sys.argv[1] if len(sys.argv) == 2 else "bbmd"
    if role == "bbmd":
        await run_bbmd()
    elif role == "broadcaster":
        run_broadcaster()
    else:
        raise SystemExit("usage: w2_bacpypes3_bbmd.py {bbmd|broadcaster}")


if __name__ == "__main__":
    asyncio.run(main())
