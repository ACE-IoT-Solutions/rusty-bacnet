"""Deterministic bacpypes3 BBMD interoperability fixture.

The FDT is deliberately empty at startup.  Foreign-device entries in this
fixture must be created by a real Register-Foreign-Device BVLC request.
"""

from __future__ import annotations

import asyncio
import os
import socket
import sys

from bacpypes3.ipv4.link import BBMDLinkLayer
from bacpypes3.pdu import IPv4Address


async def run_bbmd() -> None:
    address = os.environ.get("BACNET_BBMD_ADDRESS", "172.31.0.30/24:47809")
    bbmd = BBMDLinkLayer(IPv4Address(address))
    peer = os.environ.get("BACNET_BBMD_PEER")
    if peer:
        bbmd.add_peer(IPv4Address(peer))
    print("BBMD_READY", flush=True)
    try:
        await asyncio.Event().wait()
    finally:
        bbmd._fdt_clock_handle.cancel()
        bbmd.close()


def run_broadcaster() -> None:
    """Emit a valid I-Am as an Original-Broadcast-NPDU on the BBMD LAN."""
    interface = os.environ["BACNET_INTERFACE"]
    broadcast = os.environ["BACNET_BROADCAST"]
    port = int(os.environ.get("BACNET_PORT", "47808"))
    instance = int(os.environ.get("BACNET_DEVICE_INSTANCE", "4190222"))
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
    print(f"BROADCAST_SENT instance={instance} origin={interface}:{port}", flush=True)


async def main() -> None:
    role = sys.argv[1] if len(sys.argv) == 2 else "bbmd"
    if role == "bbmd":
        await run_bbmd()
    elif role == "broadcaster":
        run_broadcaster()
    else:
        raise SystemExit("usage: bacpypes3_bbmd.py {bbmd|broadcaster}")


if __name__ == "__main__":
    asyncio.run(main())
