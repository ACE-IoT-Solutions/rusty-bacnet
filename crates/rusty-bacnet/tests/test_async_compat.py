"""Cancellation and ownership of raw UDP fixtures on each event loop."""

import asyncio
import socket
import unittest

from async_compat import async_timeout, sock_recvfrom, sock_sendto


class DatagramTests(unittest.IsolatedAsyncioTestCase):
    async def test_cancel_or_expire_before_receiving_and_reuse_socket(self):
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as receiver, socket.socket(
            socket.AF_INET, socket.SOCK_DGRAM
        ) as sender:
            for sock in (receiver, sender):
                sock.bind(("127.0.0.1", 0))
                sock.setblocking(False)
            address = receiver.getsockname()
            waiting = asyncio.Event()

            async def receive():
                waiting.set()
                return await sock_recvfrom(loop, receiver, 128)

            pending = asyncio.create_task(receive())
            await waiting.wait()
            pending.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await pending
            with self.assertRaises(asyncio.TimeoutError):
                async with async_timeout(0):
                    await sock_recvfrom(loop, receiver, 128)
            self.assertEqual(await sock_sendto(loop, sender, b"next", address), 4)
            async with async_timeout(5):
                self.assertEqual(await sock_recvfrom(loop, receiver, 128), (b"next", sender.getsockname()))
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as rebound:
            rebound.bind(address)
