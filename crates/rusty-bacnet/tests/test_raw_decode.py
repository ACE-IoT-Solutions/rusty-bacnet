"""Installed-wheel tests for hinted raw BACnet value decoding."""

from __future__ import annotations

import inspect
import math
import unittest

from rusty_bacnet import RawTag, decode_raw_value, describe_tags


class RawValueDecodeTests(unittest.TestCase):
    def test_all_application_datatype_hints(self) -> None:
        cases = (
            (b"\x00", "null", "null", None),
            (b"\x11", "boolean", "boolean", True),
            (b"\x21\x2a", "unsigned", "unsigned", 42),
            (b"\x31\xff", "signed", "signed", -1),
            (b"\x44\x3f\xc0\x00\x00", "real", "real", 1.5),
            (b"\x55\x08\x3f\xf8\x00\x00\x00\x00\x00\x00", "double", "double", 1.5),
            (b"\x63\x00\xff\x10", "octet_string", "octet_string", b"\x00\xff\x10"),
            (b"\x73\x00hi", "character_string", "character_string", "hi"),
            (
                b"\x82\x03\xa0",
                "bit_string",
                "bit_string",
                {"unused_bits": 3, "data": b"\xa0"},
            ),
            (b"\x91\x07", "enumerated", "enumerated", 7),
            (b"\xa4\x7e\x03\x15\x06", "date", "date", (126, 3, 21, 6)),
            (b"\xb4\x0e\x1e\x00\x00", "time", "time", (14, 30, 0, 0)),
        )
        for raw, hint, tag, expected in cases:
            with self.subTest(hint=hint):
                decoded = decode_raw_value(raw, hint)
                self.assertEqual(decoded.tag, tag)
                self.assertEqual(decoded.value, expected)

        oid = decode_raw_value(b"\xc4\x00\x80\x00\x01", "object_identifier")
        self.assertEqual(oid.tag, "object_identifier")
        self.assertEqual(oid.value.object_type.to_raw(), 2)
        self.assertEqual(oid.value.instance, 1)

    def test_boolean_lvt_and_null_rules_are_explicit(self) -> None:
        self.assertFalse(decode_raw_value(b"\x10", "boolean").value)
        self.assertTrue(decode_raw_value(b"\x11", "boolean").value)
        self.assertTrue(decode_raw_value(b"\x19\x01", "boolean").value)
        self.assertFalse(decode_raw_value(b"\x19\x00", "boolean").value)

        for raw, hint in (
            (b"\x12", "boolean"),
            (b"\x15\x01", "boolean"),
            (b"\x19\x02", "boolean"),
            (b"\x01\x00", "null"),
            (b"\x05\x00", "null"),
            (b"\x09\x00", "null"),
        ):
            with self.subTest(raw=raw, hint=hint):
                with self.assertRaises(ValueError):
                    decode_raw_value(raw, hint)

    def test_context_and_proprietary_tags_cast_from_hint(self) -> None:
        real = decode_raw_value(
            b"\xfc\x14\x3f\xc0\x00\x00", "prop:1330:real"
        )
        self.assertEqual(real.tag, "real")
        self.assertEqual(real.value, 1.5)

        unsigned = decode_raw_value(b"\xf1\x14\x2a", "unsigned")
        self.assertEqual(unsigned.value, 42)

    def test_rejects_bad_hints_lengths_truncation_and_trailing_data(self) -> None:
        bad = (
            (b"", "real"),
            (b"\x44\x00\x00", "real"),
            (b"\x44\x00\x00\x00\x00\x00", "real"),
            (b"\x21\x01\x00", "unsigned"),
            (b"\x21\x01", "unknown"),
            (b"\x21\x01", "prop:nope:unsigned"),
            (b"\x21\x01", "prop:42"),
            (b"\x2e", "unsigned"),
            (b"\x55\x08\x00", "double"),
            (b"\x95\x05\x01\x02\x03\x04\x05", "enumerated"),
        )
        for raw, hint in bad:
            with self.subTest(raw=raw, hint=hint):
                with self.assertRaises(ValueError):
                    decode_raw_value(raw, hint)

    def test_rejects_noncanonical_or_invalid_primitive_encodings(self) -> None:
        bad = (
            (b"\x66\x00", "octet_string"),  # application opening LVT
            (b"\x67\x00", "octet_string"),  # application closing LVT
            (b"\xf1\x02\x2a", "unsigned"),  # extended tag number below 15
            (b"\x81\x03", "bit_string"),  # empty data with unused bits
            (b"\x82\x03\xa3", "bit_string"),  # nonzero padding bits
            (b"\xa4\x7e\x00\x00\x00", "date"),
            (b"\xb4\x18\x3c\x3c\x64", "time"),
        )
        for raw, hint in bad:
            with self.subTest(raw=raw, hint=hint):
                with self.assertRaises(ValueError):
                    decode_raw_value(raw, hint)

        # Standard wildcard and date-pattern special values remain valid.
        self.assertEqual(
            decode_raw_value(b"\xa4\xff\x0d\x20\xff", "date").value,
            (255, 13, 32, 255),
        )
        self.assertEqual(
            decode_raw_value(b"\xb4\xff\xff\xff\xff", "time").value,
            (255, 255, 255, 255),
        )

    def test_nan_real_remains_a_float(self) -> None:
        value = decode_raw_value(b"\x44\x7f\xc0\x00\x00", "real")
        self.assertTrue(math.isnan(value.value))


class TagDescriptionTests(unittest.TestCase):
    def test_extended_tag_and_length_preserve_exact_bytes(self) -> None:
        raw = b"\xfd\x14\x06abcdef"
        entries = describe_tags(raw)
        self.assertEqual(len(entries), 1)
        entry = entries[0]
        self.assertIsInstance(entry, RawTag)
        self.assertEqual(entry.tag_class, "context")
        self.assertEqual(entry.tag_number, 20)
        self.assertEqual(entry.length, 6)
        self.assertEqual(entry.header, b"\xfd\x14\x06")
        self.assertEqual(entry.content, b"abcdef")
        self.assertEqual(entry.full_tlv, raw)
        self.assertFalse(entry.opening)
        self.assertFalse(entry.closing)
        self.assertEqual(entry.depth, 0)
        self.assertEqual(
            repr(entry),
            "RawTag(tag_class=\"context\", tag_number=20, length=6, opening=false, closing=false, depth=0)",
        )
        with self.assertRaises(AttributeError):
            entry.length = 9

        two_byte_content = bytes(range(254))
        two_byte = b"\xfd\x14\xfe\x00\xfe" + two_byte_content
        two_byte_entry = describe_tags(two_byte)[0]
        self.assertEqual(two_byte_entry.length, 254)
        self.assertEqual(two_byte_entry.header, b"\xfd\x14\xfe\x00\xfe")
        self.assertEqual(two_byte_entry.content, two_byte_content)

        four_byte_content = b"x" * 65_536
        four_byte = b"\xfd\x14\xff\x00\x01\x00\x00" + four_byte_content
        four_byte_entry = describe_tags(four_byte)[0]
        self.assertEqual(four_byte_entry.length, 65_536)
        self.assertEqual(four_byte_entry.header, b"\xfd\x14\xff\x00\x01\x00\x00")
        self.assertEqual(four_byte_entry.content, four_byte_content)

    def test_nested_constructed_tags_report_depth_and_markers(self) -> None:
        raw = b"\x2e\x21\x01\xfe\x14\x19\x7f\xff\x14\x2f"
        entries = describe_tags(raw)
        self.assertEqual([entry.depth for entry in entries], [0, 1, 1, 2, 1, 0])
        self.assertEqual(
            [(entry.tag_number, entry.opening, entry.closing) for entry in entries],
            [
                (2, True, False),
                (2, False, False),
                (20, True, False),
                (1, False, False),
                (20, False, True),
                (2, False, True),
            ],
        )
        self.assertEqual(entries[0].full_tlv, b"\x2e")
        self.assertEqual(entries[4].header, b"\xff\x14")

    def test_malformed_unmatched_and_excessive_depth_are_rejected(self) -> None:
        maximum_depth = (b"\x0e" * 32) + (b"\x0f" * 32)
        self.assertEqual(len(describe_tags(maximum_depth)), 64)

        malformed = (
            b"\x25",
            b"\x21",
            b"\x65\x04",  # short length encoded through extended form
            b"\x65\xfe\x00\xfd",  # two-byte length below canonical threshold
            b"\x65\xff\x00\x00\xff\xff",  # four-byte length below threshold
            b"\x6d\xff\x00\x10\x00\x01",  # declared length above 1 MiB cap
            b"\x2f",
            b"\x2e\x3f",
            b"\x2e\x21\x01",
            (b"\x0e" * 33) + (b"\x0f" * 33),
            b"\x12",
            b"\x15\x01",
            b"\x01\x00",
            b"\x05\x00",
            b"\x66\x00",
            b"\x67\x00",
            b"\xf1\x02\x2a",
            b"\xfe\x02\x2f",
        )
        for raw in malformed:
            with self.subTest(raw=raw[:16]):
                with self.assertRaises(ValueError):
                    describe_tags(raw)

    def test_raw_decoder_enforces_tag_length_sanity_cap_before_content(self) -> None:
        # A context-tagged octet string may be cast by hint, but its declared
        # length is still subject to the shared decoder's 1 MiB resource cap.
        oversized_header = b"\x6d\xff\x00\x10\x00\x01"
        with self.assertRaises(ValueError):
            decode_raw_value(oversized_header, "octet_string")

    def test_public_signatures_are_stable(self) -> None:
        self.assertEqual(tuple(inspect.signature(decode_raw_value).parameters), ("raw", "hint"))
        self.assertEqual(tuple(inspect.signature(describe_tags).parameters), ("raw",))


if __name__ == "__main__":
    unittest.main()
