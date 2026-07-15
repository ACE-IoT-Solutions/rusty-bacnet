"""Deterministic bacpypes3 BBMD interoperability fixture."""

from __future__ import annotations

import asyncio

from bacpypes3.ipv4.link import BBMDLinkLayer
from bacpypes3.pdu import IPv4Address


async def main() -> None:
    bbmd = BBMDLinkLayer(IPv4Address("172.31.0.30/24:47809"))
    bbmd.add_peer(IPv4Address("172.31.0.31/24:47811"))
    bbmd.register_foreign_device(IPv4Address("172.31.0.32:47810"), 120)
    try:
        await asyncio.Event().wait()
    finally:
        bbmd._fdt_clock_handle.cancel()
        bbmd.close()


if __name__ == "__main__":
    asyncio.run(main())
