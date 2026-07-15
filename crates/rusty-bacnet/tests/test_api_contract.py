"""Installed-package contract checks for the rusty_bacnet extension.

These tests intentionally import the installed package. Run them from a clean
virtual environment after building/installing the wheel under test; importing
directly from Cargo build output does not exercise Python packaging.
"""

from __future__ import annotations

import inspect
import unittest

import rusty_bacnet as bacnet


class InstalledApiContractTests(unittest.TestCase):
    def test_required_public_types_are_exported(self) -> None:
        required_types = (
            "BACnetClient",
            "BACnetServer",
            "CovNotification",
            "DiscoveredDevice",
            "DirectTarget",
            "ObjectIdentifier",
            "ObjectType",
            "PropertyIdentifier",
            "PropertyValue",
            "RoutedTarget",
            "RouterInfo",
            "BdtEntry",
            "FdtEntry",
            "BacnetBvlcError",
            "BacnetNotificationLagError",
            "ManagedCOVEvent",
            "ManagedCOVEventIterator",
            "ManagedCOVSubscription",
        )

        for name in required_types:
            with self.subTest(name=name):
                exported = getattr(bacnet, name, None)
                self.assertIsNotNone(exported, f"rusty_bacnet.{name} is missing")
                self.assertTrue(
                    inspect.isclass(exported),
                    f"rusty_bacnet.{name} is not a class: {exported!r}",
                )

    def test_required_client_methods_are_callable(self) -> None:
        required_methods = (
            "stop",
            "read_property",
            "read_property_multiple",
            "write_property",
            "write_property_multiple",
            "who_is",
            "who_is_router",
            "router_snapshot",
            "read_bdt",
            "read_fdt",
            "discover",
            "discovered_devices",
            "read_property_from_device",
            "read_property_multiple_from_device",
            "write_property_to_device",
            "write_property_multiple_to_device",
            "subscribe_cov",
            "unsubscribe_cov",
            "manage_cov_subscription",
            "cov_notifications",
        )

        for name in required_methods:
            with self.subTest(name=name):
                method = getattr(bacnet.BACnetClient, name, None)
                self.assertTrue(
                    callable(method),
                    f"BACnetClient.{name} is missing or non-callable: {method!r}",
                )

    def test_core_method_signatures_match_the_public_stub(self) -> None:
        expected_parameters = {
            "read_property": (
                "self",
                "address",
                "object_id",
                "property_id",
                "array_index",
            ),
            "read_property_multiple": ("self", "address", "specs"),
            "write_property": (
                "self",
                "address",
                "object_id",
                "property_id",
                "value",
                "priority",
                "array_index",
            ),
            "discover": ("self", "timeout_ms", "low_limit", "high_limit"),
            "subscribe_cov": (
                "self",
                "address",
                "subscriber_process_identifier",
                "monitored_object_identifier",
                "confirmed",
                "lifetime",
            ),
        }

        for name, expected in expected_parameters.items():
            with self.subTest(name=name):
                method = getattr(bacnet.BACnetClient, name)
                actual = tuple(inspect.signature(method).parameters)
                self.assertEqual(actual, expected)

    def test_proprietary_identifiers_remain_numeric(self) -> None:
        object_type = bacnet.ObjectType.from_raw(128)
        property_id = bacnet.PropertyIdentifier.from_raw(512)

        self.assertEqual(object_type.to_raw(), 128)
        self.assertEqual(property_id.to_raw(), 512)

    def test_property_value_conversion_categories_are_lossless(self) -> None:
        proprietary_oid = bacnet.ObjectIdentifier(bacnet.ObjectType.from_raw(128), 77)
        values = (
            (bacnet.PropertyValue.null(), "null", None),
            (bacnet.PropertyValue.enumerated(4_000_000_000), "enumerated", 4_000_000_000),
            (
                bacnet.PropertyValue.bit_string(3, b"\xaa\xe0"),
                "bit_string",
                {"unused_bits": 3, "data": b"\xaa\xe0"},
            ),
            (
                bacnet.PropertyValue.object_identifier(proprietary_oid),
                "object_identifier",
                proprietary_oid,
            ),
            (bacnet.PropertyValue.octet_string(b"\x00\xff\x10"), "octet_string", b"\x00\xff\x10"),
        )
        for value, tag, expected in values:
            with self.subTest(tag=tag):
                self.assertEqual(value.tag, tag)
                self.assertEqual(value.value, expected)

        constructed = bacnet.PropertyValue.list([value for value, _, _ in values])
        self.assertEqual(constructed.tag, "list")
        self.assertEqual(constructed.value, [expected for _, _, expected in values])

    def test_direct_and_routed_targets_preserve_address_bytes(self) -> None:
        default_port = bacnet.DirectTarget("192.0.2.9:47808")
        self.assertEqual(default_port.mac, b"\xc0\x00\x02\x09\xba\xc0")

        direct = bacnet.DirectTarget("192.0.2.10:47809")
        self.assertEqual(direct.address, "192.0.2.10:47809")
        self.assertEqual(direct.mac, b"\xc0\x00\x02\x0a\xba\xc1")

        one_byte_dadr = bacnet.RoutedTarget(
            "198.51.100.6:47808", 2000, b"\x05"
        )
        self.assertEqual(one_byte_dadr.router_mac, b"\xc6\x33\x64\x06\xba\xc0")
        self.assertEqual(one_byte_dadr.address, b"\x05")

        routed = bacnet.RoutedTarget(
            "198.51.100.7:47810", 2001, b"\x00\x7f\x01"
        )
        self.assertEqual(routed.router, "198.51.100.7:47810")
        self.assertEqual(routed.router_mac, b"\xc6\x33\x64\x07\xba\xc2")
        self.assertEqual(routed.network, 2001)
        self.assertEqual(routed.address, b"\x00\x7f\x01")

        with self.assertRaises(AttributeError):
            direct.address = "192.0.2.11:47808"
        with self.assertRaises(AttributeError):
            routed.network = 2002

    def test_target_validation_rejects_broadcast_and_invalid_dadr(self) -> None:
        with self.assertRaises(ValueError):
            bacnet.DirectTarget("ff:ff:ff:ff:ba:c0")
        with self.assertRaises(ValueError):
            bacnet.DirectTarget("255.255.255.255:47808")
        with self.assertRaises(ValueError):
            bacnet.RoutedTarget("[::1]:47808", 10, b"\x01")
        with self.assertRaises(ValueError):
            bacnet.RoutedTarget("192.0.2.1:47808", 0, b"\x01")
        with self.assertRaises(ValueError):
            bacnet.RoutedTarget("192.0.2.1:47808", 65535, b"\x01")
        with self.assertRaises(ValueError):
            bacnet.RoutedTarget("192.0.2.1:47808", 10, b"")
        with self.assertRaises(ValueError):
            bacnet.RoutedTarget("192.0.2.1:47808", 10, b"\x01" * 256)


if __name__ == "__main__":
    unittest.main()
