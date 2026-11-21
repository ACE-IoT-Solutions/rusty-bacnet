"""Raw test sockets have at most one reader and one writer.

Await cancellation before closing or reusing the socket, as with asyncio's
native socket operations.
"""

import asyncio
import sys

if sys.version_info >= (3, 11):
    from asyncio import timeout as async_timeout
else:
    from async_timeout import timeout as async_timeout


async def _ready(loop, sock, *, reading):
    """Wait for one nonblocking socket operation without consuming a datagram."""
    fd = sock.fileno()
    ready = loop.create_future()
    add = loop.add_reader if reading else loop.add_writer
    remove = loop.remove_reader if reading else loop.remove_writer

    def wake():
        if not ready.done():
            ready.set_result(None)

    add(fd, wake)
    try:
        await ready
    finally:
        remove(fd)


async def sock_recvfrom(loop, sock, size):
    """Receive one datagram and its address, including on CPython 3.10."""
    if sys.version_info >= (3, 11):
        return await loop.sock_recvfrom(sock, size)
    if sys.platform == "win32" and isinstance(loop, asyncio.ProactorEventLoop):
        # 3.10's asyncio datagram transport uses these cancellable IOCP
        # operations internally. Public loop.sock_recvfrom arrived in 3.11.
        return await loop._proactor.recvfrom(sock, size)
    while True:
        try:
            return sock.recvfrom(size)
        except (BlockingIOError, InterruptedError):
            await _ready(loop, sock, reading=True)


async def sock_sendto(loop, sock, data, address):
    """Send one datagram using the fixture's existing nonblocking socket."""
    if sys.version_info >= (3, 11):
        return await loop.sock_sendto(sock, data, address)
    if sys.platform == "win32" and isinstance(loop, asyncio.ProactorEventLoop):
        return await loop._proactor.sendto(sock, data, addr=address)
    while True:
        try:
            return sock.sendto(data, address)
        except (BlockingIOError, InterruptedError):
            await _ready(loop, sock, reading=False)
