"""Installed-wheel coverage for server object-registration metadata."""

from __future__ import annotations

import inspect
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


class RegistrationMetadataTests(unittest.IsolatedAsyncioTestCase):
    def test_existing_positional_contracts_keep_their_defaults(self) -> None:
        expected_defaults = {
            "add_analog_input": {
                "units": 62,
                "present_value": 0.0,
                "description": "",
                "cov_increment": 0.0,
            },
            "add_analog_output": {
                "units": 62,
                "present_value": 0.0,
                "description": "",
                "cov_increment": 0.0,
            },
            "add_analog_value": {
                "units": 62,
                "present_value": 0.0,
                "description": "",
                "cov_increment": 0.0,
            },
            "add_binary_input": {"present_value": False, "description": ""},
            "add_binary_output": {"present_value": False, "description": ""},
            "add_binary_value": {"present_value": False, "description": ""},
            "add_character_string_value": {"present_value": "", "description": ""},
            "add_multistate_input": {
                "state_text": None,
                "present_value": 1,
                "description": "",
            },
            "add_multistate_output": {
                "state_text": None,
                "present_value": 1,
                "description": "",
            },
            "add_multistate_value": {
                "state_text": None,
                "present_value": 1,
                "description": "",
            },
        }
        for method_name, defaults in expected_defaults.items():
            parameters = inspect.signature(getattr(BACnetServer, method_name)).parameters
            with self.subTest(method=method_name):
                self.assertEqual(list(parameters)[:3], ["self", "instance", "name"])
                for parameter_name, default in defaults.items():
                    self.assertEqual(parameters[parameter_name].default, default)

    def test_registration_rejects_inconsistent_metadata_before_start(self) -> None:
        server = BACnetServer(810_002)
        with self.assertRaisesRegex(BacnetError, "exactly 2 entries"):
            server.add_multistate_value(1, "MSV", 2, state_text=["Only one"])
        with self.assertRaisesRegex(BacnetError, "1..=2"):
            server.add_multistate_input(2, "MSI", 2, present_value=3)
        with self.assertRaises(BacnetError):
            server.add_analog_value(3, "AV", present_value=float("nan"))
        with self.assertRaises(BacnetError):
            server.add_analog_value(4, "AV", cov_increment=-0.1)

    async def test_metadata_and_initial_values_are_visible_over_bacnet(self) -> None:
        server = BACnetServer(
            810_001,
            "Registration Metadata",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_analog_input(
            1,
            "AI",
            units=95,
            present_value=12.5,
            description="analog input",
            cov_increment=0.25,
        )
        server.add_analog_output(
            2,
            "AO",
            units=62,
            present_value=8.5,
            description="analog output",
            cov_increment=0.5,
        )
        server.add_analog_value(
            3,
            "AV",
            units=98,
            present_value=33.0,
            description="analog value",
            cov_increment=1.5,
        )
        server.add_binary_input(4, "BI", present_value=True, description="binary input")
        server.add_binary_output(5, "BO", present_value=True, description="binary output")
        server.add_binary_value(6, "BV", present_value=True, description="binary value")
        server.add_multistate_input(
            7,
            "MSI",
            2,
            state_text=["Off", "On"],
            present_value=2,
            description="multistate input",
        )
        server.add_multistate_output(
            8,
            "MSO",
            3,
            state_text=["Low", "Medium", "High"],
            present_value=3,
            description="multistate output",
        )
        server.add_multistate_value(
            9,
            "MSV",
            2,
            state_text=["Auto", "Manual"],
            present_value=2,
            description="multistate value",
        )
        server.add_character_string_value(
            10,
            "CSV",
            present_value="fallback",
            description="character string value",
        )

        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=2_000,
            ) as client:
                analog = (
                    (ObjectType.ANALOG_INPUT, 1, 95, 12.5, "analog input", 0.25),
                    (ObjectType.ANALOG_OUTPUT, 2, 62, 8.5, "analog output", 0.5),
                    (ObjectType.ANALOG_VALUE, 3, 98, 33.0, "analog value", 1.5),
                )
                for object_type, instance, units, value, description, increment in analog:
                    oid = ObjectIdentifier(object_type, instance)
                    with self.subTest(object_type=object_type):
                        self.assertEqual(
                            await client.read_property(address, oid, PropertyIdentifier.UNITS),
                            PropertyValue.enumerated(units),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.PRESENT_VALUE
                            ),
                            PropertyValue.real(value),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.DESCRIPTION
                            ),
                            PropertyValue.character_string(description),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.COV_INCREMENT
                            ),
                            PropertyValue.real(increment),
                        )
                        if object_type != ObjectType.ANALOG_INPUT:
                            self.assertEqual(
                                await client.read_property(
                                    address, oid, PropertyIdentifier.RELINQUISH_DEFAULT
                                ),
                                PropertyValue.real(value),
                            )

                binary = (
                    (ObjectType.BINARY_INPUT, 4, "binary input"),
                    (ObjectType.BINARY_OUTPUT, 5, "binary output"),
                    (ObjectType.BINARY_VALUE, 6, "binary value"),
                )
                for object_type, instance, description in binary:
                    oid = ObjectIdentifier(object_type, instance)
                    with self.subTest(object_type=object_type):
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.PRESENT_VALUE
                            ),
                            PropertyValue.enumerated(1),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.DESCRIPTION
                            ),
                            PropertyValue.character_string(description),
                        )
                        if object_type != ObjectType.BINARY_INPUT:
                            self.assertEqual(
                                await client.read_property(
                                    address, oid, PropertyIdentifier.RELINQUISH_DEFAULT
                                ),
                                PropertyValue.enumerated(1),
                            )

                multistate = (
                    (
                        ObjectType.MULTI_STATE_INPUT,
                        7,
                        2,
                        2,
                        ["Off", "On"],
                        "multistate input",
                    ),
                    (
                        ObjectType.MULTI_STATE_OUTPUT,
                        8,
                        3,
                        3,
                        ["Low", "Medium", "High"],
                        "multistate output",
                    ),
                    (
                        ObjectType.MULTI_STATE_VALUE,
                        9,
                        2,
                        2,
                        ["Auto", "Manual"],
                        "multistate value",
                    ),
                )
                for (
                    object_type,
                    instance,
                    count,
                    value,
                    state_text,
                    description,
                ) in multistate:
                    oid = ObjectIdentifier(object_type, instance)
                    with self.subTest(object_type=object_type):
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.NUMBER_OF_STATES
                            ),
                            PropertyValue.unsigned(count),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.PRESENT_VALUE
                            ),
                            PropertyValue.unsigned(value),
                        )
                        self.assertEqual(
                            await client.read_property(
                                address, oid, PropertyIdentifier.DESCRIPTION
                            ),
                            PropertyValue.character_string(description),
                        )
                        if object_type != ObjectType.MULTI_STATE_INPUT:
                            self.assertEqual(
                                await client.read_property(
                                    address, oid, PropertyIdentifier.RELINQUISH_DEFAULT
                                ),
                                PropertyValue.unsigned(value),
                            )
                        for index, text in enumerate(state_text, start=1):
                            self.assertEqual(
                                await client.read_property(
                                    address,
                                    oid,
                                    PropertyIdentifier.STATE_TEXT,
                                    array_index=index,
                                ),
                                PropertyValue.character_string(text),
                            )

                csv = ObjectIdentifier(ObjectType.CHARACTERSTRING_VALUE, 10)
                self.assertEqual(
                    await client.read_property(
                        address, csv, PropertyIdentifier.PRESENT_VALUE
                    ),
                    PropertyValue.character_string("fallback"),
                )
                self.assertEqual(
                    await client.read_property(
                        address, csv, PropertyIdentifier.RELINQUISH_DEFAULT
                    ),
                    PropertyValue.character_string("fallback"),
                )
                self.assertEqual(
                    await client.read_property(address, csv, PropertyIdentifier.DESCRIPTION),
                    PropertyValue.character_string("character string value"),
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
