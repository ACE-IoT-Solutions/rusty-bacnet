"""Installed-wheel peer quota enforcement and recovery on two real UDP sources."""
import unittest

from admission_peer import HeldWrite, expect_response, idle_counters, peer_socket, send

from rusty_bacnet import BACnetServer


class PeerAdmissionRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_two_udp_sources_peer_rejection_and_recovery(self):
        hold = HeldWrite()
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", mutation_policy=hold,
                              max_confirmed_in_flight_per_peer=1,
                              max_unconfirmed_in_flight_per_peer=1)
        server.add_analog_value(1, "Admission AV")
        with peer_socket() as first, peer_socket() as second:
            self.assertNotEqual(first.getsockname(), second.getsockname())
            await server.start()
            try:
                ip, port = (await server.local_address()).rsplit(":", 1)
                address = (ip, int(port))
                async with hold.pending(first, address):
                    self.assertEqual((await server.request_admission_counters())["confirmed_active"], 1)
                    # Same peer: resource Abort. A different peer's empty
                    # ReadProperty reaches the handler: missing required parameter.
                    await send(first, address, bytes.fromhex("00 05 02 0c"))
                    await expect_response(self, first, address, bytes.fromhex("71 02 09"))
                    await send(second, address, bytes.fromhex("00 05 03 0c"))
                    await expect_response(self, second, address, bytes.fromhex("60 03 05"))

                await expect_response(self, first, address, bytes.fromhex("20 01 0f"))
                counters = await idle_counters(server)
                self.assertEqual(counters["confirmed_peer_overloaded_total"], 1)
                self.assertEqual(counters["confirmed_global_overloaded_total"], 0)
                self.assertEqual(counters["confirmed_overloaded_total"], 1)
                self.assertEqual(counters["abort_admitted_total"], 1)
                self.assertEqual(counters["confirmed_fallback_dropped_total"], 0)
                for sock in (first, second):
                    await send(sock, address, bytes.fromhex("00 05 fa 0c"))
                    await expect_response(self, sock, address, bytes.fromhex("60 fa 05"))
                    await idle_counters(server)
            finally:
                await server.stop()
