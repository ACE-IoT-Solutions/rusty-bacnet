"""The stack's tracing events reach Python's ``logging`` (#1724).

Events go to ``rusty_bacnet.<target>``, the Rust module path with dots. The
bridge reads the logging levels at import and on ``refresh_logging()``, so
a lower level takes effect only after a refresh. Records arrive from a
forwarder thread, so each test waits for them.

The datagrams are sent to a running server's B/IP port:

- an NPDU with protocol version 2 is dropped with a warning
  (``Dropping an inbound NPDU``);
- an NPDU for a remote network is dropped at debug by a node that doesn't
  route (``Discarding routed message``).
"""
import asyncio
import logging
import socket
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetServer

# Original-Unicast-NPDU, then NPDU version 2 with no control bits.
BAD_VERSION = bytes([0x81, 0x0A, 0x00, 0x06, 0x02, 0x00])
# Original-Unicast-NPDU, then an NPDU for station 5 on network 2 (DNET 2,
# DLEN 1, DADR 5, hop count 255) carrying a Who-Is.
ROUTED = bytes([0x81, 0x0A, 0x00, 0x0D, 0x01, 0x20, 0x00, 0x02, 0x01, 0x05,
                0xFF, 0x10, 0x08])
LOGGER = "rusty_bacnet.bacnet_network"
WARNING_TEXT = "Dropping an inbound NPDU"
DEBUG_TEXT = "Discarding routed message"


class LoggingBridgeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        # Whatever an earlier test set, start from the configured levels.
        rusty_bacnet.refresh_logging()
        self.server = BACnetServer(
            device_instance=1724,
            device_name="Logging Bridge Test",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        await self.server.start()
        host, port = (await self.server.local_address()).rsplit(":", 1)
        self.address = (host, int(port))
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(("127.0.0.1", 0))

    async def asyncTearDown(self) -> None:
        self.socket.close()
        await self.server.stop()
        rusty_bacnet.refresh_logging()

    def send(self, datagram: bytes) -> None:
        self.socket.sendto(datagram, self.address)

    async def wait_for(self, records: list, text: str) -> logging.LogRecord:
        for _ in range(500):
            for record in records:
                if text in record.getMessage():
                    return record
            await asyncio.sleep(0.01)
        self.fail(f"no record with {text!r} in {[r.getMessage() for r in records]}")

    async def test_a_warning_from_the_stack_reaches_logging(self) -> None:
        with self.assertLogs(LOGGER, "WARNING") as captured:
            self.send(BAD_VERSION)
            record = await self.wait_for(captured.records, WARNING_TEXT)
        self.assertEqual(record.levelno, logging.WARNING)
        self.assertTrue(record.name.startswith(LOGGER + "."), record.name)
        self.assertEqual(record.bacnet_target, record.name.removeprefix(
            "rusty_bacnet.").replace(".", "::"))
        # The event's fields are in the message and on the record.
        self.assertIn("discard", record.bacnet_fields)
        self.assertIn(" discard=", record.getMessage())
        self.assertTrue(record.pathname.endswith(".rs"), record.pathname)

    async def test_a_lower_level_takes_effect_after_refresh_logging(self) -> None:
        with self.assertLogs(LOGGER, "DEBUG") as captured:
            # DEBUG on the logger now, but the bridge still has the levels it
            # read before: the debug event isn't forwarded. The warning sent
            # after it comes through the same receive path, so once it's
            # here the debug event would be too.
            self.send(ROUTED)
            self.send(BAD_VERSION)
            await self.wait_for(captured.records, WARNING_TEXT)
            texts = [record.getMessage() for record in captured.records]
            self.assertFalse(any(DEBUG_TEXT in text for text in texts), texts)

            rusty_bacnet.refresh_logging()
            self.send(ROUTED)
            record = await self.wait_for(captured.records, DEBUG_TEXT)
        self.assertEqual(record.levelno, logging.DEBUG)

    async def test_a_handler_may_call_back_into_rusty_bacnet(self) -> None:
        calls = []

        class CallBack(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                # On the forwarder thread, with the GIL: no lock of the
                # bridge is held, so this neither deadlocks nor stops it.
                rusty_bacnet.refresh_logging()
                calls.append(record.getMessage())

        # assertLogs puts the logger's own handlers back when it ends,
        # which removes this one.
        with self.assertLogs(LOGGER, "WARNING") as captured:
            logging.getLogger(LOGGER).addHandler(CallBack())
            self.send(BAD_VERSION)
            await self.wait_for(captured.records, WARNING_TEXT)
            self.send(BAD_VERSION)
            for _ in range(500):
                if len(calls) >= 2:
                    break
                await asyncio.sleep(0.01)
        self.assertEqual(len(calls), 2, calls)

    def test_trace_has_a_level_name(self) -> None:
        self.assertEqual(logging.getLevelName(5), "TRACE")


if __name__ == "__main__":
    unittest.main()
