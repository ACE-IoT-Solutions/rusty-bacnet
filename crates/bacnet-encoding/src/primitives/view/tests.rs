//! The borrowed views: independent wire cases for each application tag,
//! the refusals, and (with alloc) agreement with the owned decoder.

use super::*;
use bacnet_types::enums::ObjectType;

fn value(data: &[u8]) -> (ApplicationValueRef<'_>, usize) {
    decode_application_value_ref(data, 0).expect("the value decodes")
}

/// The text of the error `data` is refused with.
fn refusal(data: &[u8]) -> String {
    decode_application_value_ref(data, 0)
        .expect_err("the value is refused")
        .to_string()
}

// Hand-tabulated from Clauses 20.2.1-20.2.14: tag octet, then contents.
#[test]
fn each_application_tag_reads_in_place() {
    let text = [0x75, 0x05, 0x00, b'a', b'b', b'c', b'd'];
    let ApplicationValueRef::CharacterString(read) = value(&text).0 else {
        panic!("a CharacterString");
    };
    assert_eq!(
        (read.charset(), read.as_str(), read.bytes()),
        (charset::UTF8, Some("abcd"), &b"abcd"[..])
    );
    // The string borrows the input's octets.
    assert!(core::ptr::eq(read.bytes().as_ptr(), text[3..].as_ptr()));

    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    for (wire, expected) in [
        (&[0x00][..], ApplicationValueRef::Null),
        (&[0x10], ApplicationValueRef::Boolean(false)),
        (&[0x11], ApplicationValueRef::Boolean(true)),
        (&[0x22, 0x01, 0x00], ApplicationValueRef::Unsigned(256)),
        (&[0x31, 0xFF], ApplicationValueRef::Signed(-1)),
        (
            &[0x44, 0x42, 0x91, 0x00, 0x00],
            ApplicationValueRef::Real(72.5),
        ),
        (
            &[0x55, 0x08, 0x40, 0x09, 0x21, 0xFB, 0x54, 0x44, 0x2D, 0x18],
            ApplicationValueRef::Double(core::f64::consts::PI),
        ),
        (
            &[0x62, 0xAB, 0xCD],
            ApplicationValueRef::OctetString(&[0xAB, 0xCD]),
        ),
        (
            &[0x82, 0x04, 0xF0],
            ApplicationValueRef::BitString {
                unused_bits: 4,
                data: &[0xF0],
            },
        ),
        (&[0x91, 0x03], ApplicationValueRef::Enumerated(3)),
        (
            &[0xA4, 0x7E, 0x0A, 0x03, 0xFF],
            ApplicationValueRef::Date(Date {
                year: 126,
                month: 10,
                day: 3,
                day_of_week: 0xFF,
            }),
        ),
        (
            &[0xB4, 0x0E, 0x1E, 0x00, 0x00],
            ApplicationValueRef::Time(Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            }),
        ),
        (
            &[0xC4, 0x02, 0x00, 0x04, 0xD2],
            ApplicationValueRef::ObjectIdentifier(device),
        ),
    ] {
        assert_eq!(value(wire), (expected, wire.len()), "{wire:02X?}");
    }
}

#[test]
fn a_value_after_another_starts_where_it_ends() {
    // Unsigned 7, then a Latin-1 CharacterString, which stays unconverted.
    let data = [0x21, 0x07, 0x72, 0x05, 0xE9];
    let (first, next) = decode_application_value_ref(&data, 0).unwrap();
    assert_eq!((first, next), (ApplicationValueRef::Unsigned(7), 2));
    let (second, end) = decode_application_value_ref(&data, next).unwrap();
    let ApplicationValueRef::CharacterString(text) = second else {
        panic!("a CharacterString");
    };
    assert_eq!(
        (text.charset(), text.as_str(), text.bytes(), end),
        (charset::ISO_8859_1, None, &[0xE9][..], 5)
    );
}

#[test]
fn malformed_values_are_refused() {
    for (wire, text) in [
        (
            &[0x09, 0x01][..],
            "decoding error at offset 0: expected an application-tagged value, found a context tag",
        ),
        (
            &[0x01, 0x00],
            "decoding error at offset 0: application NULL must have no contents",
        ),
        (
            &[0x45, 0x05, 0x42, 0x90],
            "decoding error at offset 0: application REAL has 5 contents octets, expected 4",
        ),
        (
            &[0x25, 0x09, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            "decoding error at offset 2: unsigned requires 1-8 bytes, got 9",
        ),
        (
            &[0x95, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "decoding error at offset 2: ENUMERATED exceeds u32",
        ),
        (
            &[0x81, 0x08],
            "decoding error at offset 1: BitString unused_bits must be 0-7, got 8",
        ),
        (
            &[0x72, 0x06, b'a'],
            "decoding error at offset 1: unknown charset: 6",
        ),
        (
            &[0x73, 0x00, b'a', 0xFF],
            "decoding error at offset 2: invalid UTF-8 after 1 valid octets",
        ),
        (
            &[0xD1, 0x00],
            "decoding error at offset 0: unknown application tag number 13",
        ),
        (&[0x22, 0x01], "buffer too short: need 3 bytes, have 2"),
    ] {
        assert_eq!(refusal(wire), text, "{wire:02X?}");
    }
}

/// A fault inside the contents is reported at its offset in the data, past
/// the header and any value before it, not from the contents' start.
#[test]
fn content_faults_index_the_data() {
    let at = |data: &[u8], offset| {
        decode_application_value_ref(data, offset)
            .unwrap_err()
            .decode_fault()
    };
    // UTF-8 X'C3' then '(': the fault is the octet after the character set.
    assert_eq!(
        at(&[0x73, 0x00, 0xC3, 0x28], 0),
        Some((2, DecodingKind::InvalidEncoding))
    );
    // An Unsigned of no octets, after an Unsigned 7.
    assert_eq!(
        at(&[0x21, 0x07, 0x20], 2),
        Some((3, DecodingKind::InvalidEncoding))
    );
    // A BIT STRING whose unused-bit count is 9, after a NULL.
    assert_eq!(
        at(&[0x00, 0x81, 0x09], 1),
        Some((2, DecodingKind::InvalidEncoding))
    );
}

#[test]
fn character_string_contents_need_a_defined_character_set() {
    assert!(CharacterStringRef::decode(&[]).is_err());
    for charset in 0..=5 {
        assert_eq!(
            CharacterStringRef::decode(&[charset]).unwrap().charset(),
            charset
        );
    }
    assert!(CharacterStringRef::decode(&[6]).is_err());
    // Another character set keeps its octets, even ones not valid UTF-8.
    let ucs2 = CharacterStringRef::decode(&[charset::UCS2, 0x00, 0xE9]).unwrap();
    assert_eq!((ucs2.as_str(), ucs2.contents()), (None, &[4, 0, 0xE9][..]));
}

/// The view reads what the owned decoder reads, at the same offsets, and
/// refuses what it refuses with the same error.
#[cfg(feature = "alloc")]
#[test]
fn the_view_agrees_with_the_owned_decoder() {
    use super::super::decode_application_value;
    use bacnet_types::primitives::PropertyValue;

    let cases: &[&[u8]] = &[
        &[0x00],
        &[0x11],
        &[0x10],
        &[0x21, 0x00],
        &[0x25, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0x34, 0x80, 0x00, 0x00, 0x00],
        &[0x44, 0x7F, 0xC0, 0x00, 0x00],
        &[0x55, 0x08, 0, 0, 0, 0, 0, 0, 0, 1],
        &[0x60],
        &[0x63, 1, 2, 3],
        &[0x71, 0x00],
        &[0x74, 0x00, 0xC3, 0xA9, b'!'],
        &[0x72, 0x05, 0xE9],
        &[0x81, 0x00],
        &[0x83, 0x07, 0xFF, 0x80],
        &[0x94, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0xA4, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0xB4, 0x17, 0x3B, 0x3B, 0x63],
        &[0xC4, 0xFF, 0xFF, 0xFF, 0xFF],
        // Refused by both.
        &[],
        &[0x01, 0x00],
        &[0x12],
        &[0x20],
        &[0x35, 0x05, 1, 2, 3, 4, 5],
        &[0x43, 0x42, 0x90, 0x00],
        &[0x45, 0x05, 0x42],
        &[0x95, 0x05, 1, 0, 0, 0, 0],
        &[0x80],
        &[0x81, 0x09],
        &[0xA5, 0x05, 1, 2, 3, 4, 5],
        &[0xB3, 1, 2, 3],
        &[0xC2, 1, 2],
        &[0xE0],
        &[0x62, 0xAB],
        &[0x70],
        &[0x72, 0x09, b'x'],
        &[0x72, 0x00, 0xFF],
    ];
    for wire in cases {
        let owned = decode_application_value(wire, 0);
        let view = decode_application_value_ref(wire, 0);
        match (&owned, &view) {
            (Ok((owned, owned_end)), Ok((view, view_end))) => {
                assert_eq!(owned_end, view_end, "{wire:02X?}");
                let converted = match *view {
                    ApplicationValueRef::Null => PropertyValue::Null,
                    ApplicationValueRef::Boolean(v) => PropertyValue::Boolean(v),
                    ApplicationValueRef::Unsigned(v) => PropertyValue::Unsigned(v),
                    ApplicationValueRef::Signed(v) => PropertyValue::Signed(v),
                    ApplicationValueRef::Real(v) => PropertyValue::Real(v),
                    ApplicationValueRef::Double(v) => PropertyValue::Double(v),
                    ApplicationValueRef::OctetString(v) => PropertyValue::OctetString(v.to_vec()),
                    ApplicationValueRef::CharacterString(v) => PropertyValue::CharacterString(
                        super::super::decode_character_string(v.contents()).unwrap(),
                    ),
                    ApplicationValueRef::BitString { unused_bits, data } => {
                        PropertyValue::BitString {
                            unused_bits,
                            data: data.to_vec(),
                        }
                    }
                    ApplicationValueRef::Enumerated(v) => PropertyValue::Enumerated(v),
                    ApplicationValueRef::Date(v) => PropertyValue::Date(v),
                    ApplicationValueRef::Time(v) => PropertyValue::Time(v),
                    ApplicationValueRef::ObjectIdentifier(v) => PropertyValue::ObjectIdentifier(v),
                };
                // NaN payloads compare by their bits.
                assert_eq!(
                    format!("{owned:?}"),
                    format!("{converted:?}"),
                    "{wire:02X?}"
                );
            }
            (Err(owned), Err(view)) => {
                // The same fault and wording; the view reports a fault inside
                // the contents at its offset in the data, the owned decoder
                // from the contents' start.
                let (owned_fault, view_fault) = (owned.decode_fault(), view.decode_fault());
                let content_start = tags::decode_tag(wire, 0).map_or(0, |(_, start)| start);
                match (owned, view) {
                    (Error::Decoding(owned), Error::Decoding(view)) => {
                        assert_eq!(owned.message().to_string(), view.message().to_string());
                        assert_eq!(owned.kind, view.kind, "{wire:02X?}");
                        assert!(
                            view.offset == owned.offset
                                || view.offset == owned.offset + content_start,
                            "{wire:02X?}: {owned_fault:?} {view_fault:?}"
                        );
                    }
                    _ => assert_eq!(owned.to_string(), view.to_string(), "{wire:02X?}"),
                }
            }
            _ => panic!("{wire:02X?}: owned {owned:?}, view {view:?}"),
        }
    }
}
