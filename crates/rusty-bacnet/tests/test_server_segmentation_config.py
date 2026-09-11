"""Focused coverage for Python server segmentation configuration."""

from __future__ import annotations

import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet as bacnet


class ServerSegmentationConfigurationTests(unittest.TestCase):
    def test_signature_stub_default_and_validation(self) -> None:
        parameter = inspect.signature(bacnet.BACnetServer).parameters[
            "segmentation_supported"
        ]
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(parameter.default)
        self.assertIn(
            "segmentation_supported: Optional[Segmentation] = None",
            Path(bacnet.__file__).with_suffix(".pyi").read_text(),
        )

        for offset, mode in enumerate(
            (
                bacnet.Segmentation.BOTH,
                bacnet.Segmentation.TRANSMIT,
                bacnet.Segmentation.RECEIVE,
                bacnet.Segmentation.NONE,
            )
        ):
            bacnet.BACnetServer(4_180_100 + offset, segmentation_supported=mode)

        with self.assertRaisesRegex(ValueError, "segmentation_supported must be"):
            bacnet.BACnetServer(
                4_180_104,
                segmentation_supported=bacnet.Segmentation.from_raw(4),
            )

    def test_device_metadata_matches_runtime_mode(self) -> None:
        async def run() -> None:
            enabled = bacnet.BACnetServer(
                4_180_105,
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                segmentation_supported=bacnet.Segmentation.BOTH,
            )
            await enabled.start()
            try:
                device = bacnet.ObjectIdentifier(bacnet.ObjectType.DEVICE, 4_180_105)
                expected = {
                    bacnet.PropertyIdentifier.SEGMENTATION_SUPPORTED: bacnet.Segmentation.BOTH.to_raw(),
                    bacnet.PropertyIdentifier.APDU_SEGMENT_TIMEOUT: 5_000,
                    bacnet.PropertyIdentifier.MAX_SEGMENTS_ACCEPTED: 65,
                }
                for property_id, expected_value in expected.items():
                    actual = await enabled.read_property(device, property_id)
                    self.assertEqual(actual.value, expected_value)
            finally:
                await enabled.stop()

            disabled = bacnet.BACnetServer(
                4_180_106,
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            )
            await disabled.start()
            try:
                device = bacnet.ObjectIdentifier(bacnet.ObjectType.DEVICE, 4_180_106)
                segmentation = await disabled.read_property(
                    device, bacnet.PropertyIdentifier.SEGMENTATION_SUPPORTED
                )
                self.assertEqual(segmentation.value, bacnet.Segmentation.NONE.to_raw())
                for property_id in (
                    bacnet.PropertyIdentifier.APDU_SEGMENT_TIMEOUT,
                    bacnet.PropertyIdentifier.MAX_SEGMENTS_ACCEPTED,
                ):
                    with self.assertRaises(bacnet.BacnetProtocolError):
                        await disabled.read_property(device, property_id)
            finally:
                await disabled.stop()

        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
