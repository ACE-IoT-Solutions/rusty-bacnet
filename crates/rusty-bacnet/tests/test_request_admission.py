"""Installed native artifact admission tests; loopback UDP only, no hardware."""
from __future__ import annotations

import ast
import inspect
import unittest
from pathlib import Path
from typing import Any

from admission_peer import HeldWrite, expect_response, idle_counters, peer_socket, send

import rusty_bacnet
from rusty_bacnet import BACnetServer


FIELDS = {
    "recovery_active", "recovery_admitted_total", "recovery_overloaded_total",
    "confirmed_global_overloaded_total", "confirmed_peer_overloaded_total",
    "unconfirmed_global_overloaded_total", "unconfirmed_peer_overloaded_total",
    "confirmed_active", "confirmed_admitted_total", "confirmed_overloaded_total",
    "confirmed_shutdown_rejected_total", "unconfirmed_active",
    "unconfirmed_admitted_total", "unconfirmed_overloaded_total",
    "unconfirmed_shutdown_rejected_total", "abort_active", "abort_admitted_total",
    "confirmed_fallback_dropped_total", "abort_shutdown_rejected_total",
}


class AdmissionSignatureTests(unittest.TestCase):
    def test_installed_stub_defaults_keyword_only_and_counter_fields(self):
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        tree = ast.parse(stub.read_text())
        classes = {n.name: n for n in tree.body if isinstance(n, ast.ClassDef)}
        server = classes["BACnetServer"]
        constructor = next(n for n in server.body if isinstance(n, ast.FunctionDef) and n.name == "__init__")
        defaults = dict(zip((a.arg for a in constructor.args.kwonlyargs), constructor.args.kw_defaults))
        signature = inspect.signature(BACnetServer)
        for name, value in [("max_confirmed_in_flight", 64), ("max_unconfirmed_in_flight", 32),
                            ("max_confirmed_in_flight_per_peer", 16), ("max_unconfirmed_in_flight_per_peer", 8),
                            ("confirmed_recovery_reserve", 4), ("max_recovery_in_flight_per_peer", 1)]:
            self.assertEqual(signature.parameters[name].kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertEqual(signature.parameters[name].default, value)
            default = defaults[name]
            assert default is not None
            self.assertEqual(ast.literal_eval(default), value)
        fields = {n.target.id for n in classes["RequestAdmissionCounters"].body if isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name)}
        self.assertEqual(fields, FIELDS)
        method = next(n for n in server.body if isinstance(n, ast.FunctionDef) and n.name == "request_admission_counters")
        assert method.returns is not None
        self.assertEqual(ast.unparse(method.returns), "Awaitable[RequestAdmissionCounters]")

    def test_invalid_limits_rejected_at_constructor_for_all_transports(self):
        for transport in ["bip", "ipv6", "sc", "mstp"]:
            for name in ["max_confirmed_in_flight", "max_unconfirmed_in_flight",
                         "max_confirmed_in_flight_per_peer", "max_unconfirmed_in_flight_per_peer"]:
                for value, error in [(0, ValueError), (-1, OverflowError), (1 << 200, OverflowError), ((1 << 64) - 1, ValueError)]:
                    with self.subTest(transport=transport, name=name, value=value):
                        with self.assertRaises(error):
                            invalid: dict[str, Any] = {name: value}
                            BACnetServer(123, transport=transport, **invalid)
                with self.assertRaises(TypeError):
                    invalid = {name: 1.5}
                    BACnetServer(123, transport=transport, **invalid)
                valid: dict[str, Any] = {name: 2}
                if name == "max_confirmed_in_flight":
                    valid["confirmed_recovery_reserve"] = 0
                BACnetServer(123, transport=transport, **valid,
                             sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")


class AdmissionRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.hold = HeldWrite()
        self.server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1",
                                  max_confirmed_in_flight=1, max_unconfirmed_in_flight=2,
                                  confirmed_recovery_reserve=0, mutation_policy=self.hold)
        self.server.add_analog_value(1, "Admission AV")
        self.sock = peer_socket()
        self.addCleanup(self.sock.close)

    async def asyncTearDown(self):
        await self.server.stop()

    async def test_prestart_stopped_errors_and_zero_start_snapshot(self):
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await self.server.request_admission_counters()
        await self.server.start()
        counters = await self.server.request_admission_counters()
        self.assertEqual(set(counters), FIELDS)
        self.assertTrue(all(v == 0 for v in counters.values()))
        await self.server.stop()
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await self.server.request_admission_counters()

    async def test_real_udp_overload_counters_and_recovery(self):
        await self.server.start()
        ip, port = (await self.server.local_address()).rsplit(":", 1)
        address = (ip, int(port))
        async with self.hold.pending(self.sock, address):
            counters = await self.server.request_admission_counters()
            self.assertEqual(counters["confirmed_active"], 1)
            await send(self.sock, address, bytes.fromhex("00 05 02 0c"))
            # The only confirmed slot is held, so we see a resource Abort on wire.
            await expect_response(self, self.sock, address, bytes.fromhex("71 02 09"))
            counters = await self.server.request_admission_counters()
            self.assertEqual(counters["confirmed_active"], 1)
            self.assertLessEqual(counters["abort_active"], 8)

        await expect_response(self, self.sock, address, bytes.fromhex("20 01 0f"))
        idle = await idle_counters(self.server)
        self.assertEqual(set(idle), FIELDS)
        self.assertTrue(all(type(v) is int and v >= 0 for v in idle.values()))
        self.assertEqual(idle["confirmed_admitted_total"], 1)
        self.assertEqual(idle["confirmed_overloaded_total"], 1)
        self.assertEqual(idle["confirmed_global_overloaded_total"], 1)
        self.assertEqual(idle["abort_admitted_total"], 1)
        self.assertEqual(idle["confirmed_fallback_dropped_total"], 0)
        # After release the empty ReadProperty reaches its handler, which
        # rejects it as missing a required parameter.
        await send(self.sock, address, bytes.fromhex("00 05 fa 0c"))
        await expect_response(self, self.sock, address, bytes.fromhex("60 fa 05"))
        counters = await idle_counters(self.server)
        self.assertEqual(counters["confirmed_admitted_total"], 2)
        self.assertEqual(counters["confirmed_overloaded_total"], 1)
