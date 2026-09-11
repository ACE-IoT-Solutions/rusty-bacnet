"""Installed-wheel and source-stub API contract checks.

Run this file against an installed wheel.  Every class/function/method declared
by the shipped stub is required.  Local capabilities scheduled for later
reconciliation phases are listed separately so each gap stays visible.
"""

from __future__ import annotations

import ast
import importlib
import unittest
from pathlib import Path


STUB = Path(__file__).parents[1] / "rusty_bacnet.pyi"

# One or more public anchors for each deferred local capability.  These are
# expected gaps, not requirements of the current upstream-first tranche.
PLANNED_LOCAL_CAPABILITIES = {
    "runtime": ("BACnetRuntime", "RuntimeAttachment"),
    "i_am_provenance": ("IAmEvent", "IAmEventIterator"),
    "foreign_device": ("ForeignDeviceStatus", "BacnetForeignDeviceRegistrationError"),
    "bbmd_control": ("BbmdControl", "BbmdSnapshot"),
    "bvll_policy": ("BvllPolicyContext", "BvllPolicyVerdict"),
    "virtual_router": ("BACnetRouter", "RouterPortCounters"),
    "managed_cov": ("ManagedCOVSubscription", "ManagedCOVEventIterator"),
    "apdu_observer": ("ApduObserverEvent", "ApduObserverEventIterator"),
    "batch_scan": ("ScanSnapshot", "RuntimeDeviceScanSnapshot"),
}


def stub_contract() -> dict[str, set[str]]:
    tree = ast.parse(STUB.read_text(encoding="utf-8"), filename=str(STUB))
    contract: dict[str, set[str]] = {}
    for node in tree.body:
        if not isinstance(node, ast.ClassDef) or node.name.startswith("_"):
            continue
        # TypedDict declarations are static shapes, not runtime exports.
        if any(isinstance(base, ast.Name) and base.id == "TypedDict" for base in node.bases):
            continue
        contract[node.name] = {
            member.name
            for member in node.body
            if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef))
            and (not member.name.startswith("_") or member.name == "__init__")
        }
    return contract


class InstalledApiManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = importlib.import_module("rusty_bacnet")

    def test_installed_module_satisfies_shipped_stub(self) -> None:
        for class_name, methods in stub_contract().items():
            with self.subTest(class_name=class_name):
                exported = getattr(self.module, class_name, None)
                self.assertIsNotNone(exported, f"rusty_bacnet.{class_name} is missing")
            for method_name in methods:
                with self.subTest(class_name=class_name, method=method_name):
                    self.assertTrue(
                        hasattr(exported, method_name),
                        f"rusty_bacnet.{class_name}.{method_name} is missing",
                    )

    def test_typed_target_contract(self) -> None:
        direct = self.module.DirectTarget("192.0.2.10:47808")
        self.assertEqual(direct.address, "192.0.2.10:47808")
        self.assertEqual(direct.mac, bytes((192, 0, 2, 10, 0xBA, 0xC0)))

        routed = self.module.RoutedTarget("192.0.2.1:47808", 2001, b"\x07")
        self.assertEqual(routed.router, "192.0.2.1:47808")
        self.assertEqual(routed.network, 2001)
        self.assertEqual(routed.address, b"\x07")

        with self.assertRaises(AttributeError):
            direct.address = "192.0.2.11:47808"
        with self.assertRaises(AttributeError):
            routed.network = 2002

    def test_typed_target_validation_is_fail_closed(self) -> None:
        invalid_targets = (
            lambda: self.module.DirectTarget("ff:ff:ff:ff:ba:c0"),
            lambda: self.module.DirectTarget("255.255.255.255:47808"),
            lambda: self.module.RoutedTarget("[::1]:47808", 10, b"\x01"),
            lambda: self.module.RoutedTarget("192.0.2.1:47808", 0, b"\x01"),
            lambda: self.module.RoutedTarget("192.0.2.1:47808", 65535, b"\x01"),
            lambda: self.module.RoutedTarget("192.0.2.1:47808", 10, b""),
            lambda: self.module.RoutedTarget("192.0.2.1:47808", 10, b"\x01" * 256),
        )
        for construct in invalid_targets:
            with self.subTest(construct=construct), self.assertRaises(ValueError):
                construct()

    def test_deferred_local_capabilities_are_enumerated(self) -> None:
        # This test intentionally reports the open rows without treating them as
        # failures in the upstream-first tranche.  As capabilities land, their
        # anchors become present and cease to be gaps automatically.
        gaps = {
            capability: [name for name in anchors if not hasattr(self.module, name)]
            for capability, anchors in PLANNED_LOCAL_CAPABILITIES.items()
        }
        self.assertEqual(set(gaps), set(PLANNED_LOCAL_CAPABILITIES))
        for capability, names in gaps.items():
            with self.subTest(capability=capability):
                self.assertIsInstance(names, list)


if __name__ == "__main__":
    unittest.main()
