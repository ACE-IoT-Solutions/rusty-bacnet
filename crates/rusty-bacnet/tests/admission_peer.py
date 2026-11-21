"""A loopback peer and a server mutation policy for controlling request admission."""
import asyncio
import socket
import threading
from contextlib import asynccontextmanager

from async_compat import async_timeout, sock_recvfrom, sock_sendto
from bvll_peer import bvll


def peer_socket():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", 0))
    sock.setblocking(False)
    return sock


def packet(apdu):
    return bvll(0x0a, b"\x01\x00" + apdu)


async def send(sock, address, apdu):
    await sock_sendto(asyncio.get_running_loop(), sock, packet(apdu), address)


async def expect_response(test, sock, address, apdu):
    async with async_timeout(2):
        reply = await sock_recvfrom(asyncio.get_running_loop(), sock, 2048)
    test.assertEqual(reply, (packet(apdu), address))


async def idle_counters(server):
    async with async_timeout(2):
        while True:
            counters = await server.request_admission_counters()
            if counters["confirmed_active"] == counters["abort_active"] == 0:
                return counters
            await asyncio.sleep(0)


class HeldWrite:
    """Keep one admitted WriteProperty on the native policy worker until released."""

    def __init__(self):
        self.loop = asyncio.get_running_loop()
        self.entered = asyncio.Event()
        self.release = threading.Event()

    def __call__(self, context):
        self.loop.call_soon_threadsafe(self.entered.set)
        self.release.wait()
        return True

    @asynccontextmanager
    async def pending(self, sock, address):
        # WriteProperty AV 1 present-value = Real 1.0, invoke ID 1.
        write = bytes.fromhex("00 05 01 0f 0c 00800001 19 55 3e 44 3f800000 3f")
        try:
            await send(sock, address, write)
            async with async_timeout(2):
                await self.entered.wait()
            yield
        finally:
            self.release.set()
