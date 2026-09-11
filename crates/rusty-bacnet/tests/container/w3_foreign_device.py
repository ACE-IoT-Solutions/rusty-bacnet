"""Pinned-bacpypes3 foreign device used by the W3 BBMD acceptance fixture."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

from bacpypes3.comm import Client, bind
from bacpypes3.ipv4 import IPv4DatagramServer
from bacpypes3.ipv4.bvll import BVLLCodec
from bacpypes3.ipv4.service import BIPForeign, UDPMultiplexer
from bacpypes3.pdu import IPv4Address, LocalBroadcast, PDU


SIDE = os.environ["W3_SIDE"]
LOCAL_ADDRESS = os.environ["BACNET_LOCAL_ADDRESS"]
BBMD_ADDRESS = os.environ["BACNET_BBMD_ADDRESS"]
TTL = int(os.environ.get("BACNET_FOREIGN_DEVICE_TTL", "1"))
EXPECT_REJECT = os.environ.get("W3_EXPECT_REJECT", "0") == "1"

TOKENS = {
    "ALLOW_A": b"\x01\x00W3:ALLOW:A",
    "ALLOW_B": b"\x01\x00W3:ALLOW:B",
    "CUT": b"\x01\x00W3:CUT",
    "DROP": b"\x01\x00W3:DROP",
}
COMMANDS = {
    Path("/tmp/send-allow-a"): "ALLOW_A",
    Path("/tmp/send-allow-b"): "ALLOW_B",
    Path("/tmp/send-cut"): "CUT",
    Path("/tmp/send-drop"): "DROP",
}


def emit(message: str) -> None:
    print(message, flush=True)


class Collector(Client[PDU]):
    async def confirmation(self, pdu: PDU) -> None:
        payload = bytes(pdu.pduData)
        for token, expected in TOKENS.items():
            if payload != expected:
                continue
            source = str(pdu.pduSource)
            if not source:
                raise AssertionError(f"side {SIDE} token {token} had no forwarded origin")
            emit(f"FD_RECEIVED side={SIDE} token={token} source={source}")
            return


class RoutedForeignLinkLayer(BIPForeign):
    """bacpypes3 foreign link without an unused local-broadcast socket.

    Rootless Podman's carrier prefix is deliberately wider than the BACnet
    /24s.  A foreign device communicates only through its BBMD, so it does not
    need bacpypes3's second UDP listener on the logical /24 broadcast address.
    """

    def __init__(self, local_address: IPv4Address) -> None:
        super().__init__()
        self.codec = BVLLCodec()
        self.multiplexer = UDPMultiplexer()
        self.server = IPv4DatagramServer(local_address, no_broadcast=True)
        bind(self, self.codec, self.multiplexer.annexJ)
        bind(self.multiplexer, self.server)

    def close(self) -> None:
        self.server.close()


async def await_registration(link: RoutedForeignLinkLayer) -> int:
    async with asyncio.timeout(10):
        while link.bbmdRegistrationStatus in (-2, -1):
            await asyncio.sleep(0.05)
    return link.bbmdRegistrationStatus


async def main() -> None:
    collector = Collector()
    link = RoutedForeignLinkLayer(IPv4Address(LOCAL_ADDRESS))
    bind(collector, link)
    link.register(IPv4Address(BBMD_ADDRESS), TTL)

    status = await await_registration(link)
    if EXPECT_REJECT:
        if status != 0x0030:
            raise AssertionError(f"policy registration rejection mismatch: 0x{status:04x}")
        emit(f"FD_REJECTED side={SIDE} code=0x{status:04x}")
        link.close()
        return
    if status != 0:
        raise AssertionError(f"foreign-device registration failed: 0x{status:04x}")
    emit(f"FD_REGISTERED side={SIDE} bbmd={BBMD_ADDRESS} ttl={TTL}")

    try:
        async with asyncio.timeout(75):
            while True:
                for path, token in COMMANDS.items():
                    if not path.exists():
                        continue
                    path.unlink()
                    await collector.request(PDU(TOKENS[token], destination=LocalBroadcast()))
                    emit(f"FD_SENT side={SIDE} token={token}")
                await asyncio.sleep(0.05)
    finally:
        link.close()


if __name__ == "__main__":
    asyncio.run(main())
