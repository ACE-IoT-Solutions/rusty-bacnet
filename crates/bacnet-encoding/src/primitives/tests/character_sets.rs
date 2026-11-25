use super::*;

#[test]
fn ucs2_decode_ascii() {
    // UCS-2 BE for "AB": 0x00 0x41 0x00 0x42
    let data = [charset::UCS2, 0x00, 0x41, 0x00, 0x42];
    let result = decode_character_string(&data).unwrap();
    assert_eq!(result, "AB");
}

#[test]
fn ucs2_decode_non_ascii() {
    // UCS-2 BE for "é" (U+00E9): 0x00 0xE9
    let data = [charset::UCS2, 0x00, 0xE9];
    let result = decode_character_string(&data).unwrap();
    assert_eq!(result, "é");
}

#[test]
fn owned_ucs4_decodes_big_endian_unicode_scalars() {
    for text in ["", "\0", "Ω", "🦀", "\u{10ffff}", "\u{feff}A"] {
        let mut bytes = vec![charset::UCS4];
        for character in text.chars() {
            bytes.extend_from_slice(&u32::from(character).to_be_bytes());
        }
        assert_eq!(decode_character_string(&bytes).unwrap(), text);
    }
}

#[test]
fn malformed_ucs4_reports_the_borrowed_validators_offset() {
    for (content, offset) in [
        (&[charset::UCS4, 0, 0, 1][..], 1),
        (&[charset::UCS4, 0, 0, 0, 1, 0, 0, 0xd8, 0], 5),
        (&[charset::UCS4, 0, 0x11, 0, 0], 1),
    ] {
        let expected = Some((offset, DecodingKind::InvalidEncoding));
        assert_eq!(
            CharacterStringRef::decode(content)
                .unwrap_err()
                .decode_fault(),
            expected
        );
        assert_eq!(
            decode_character_string(content).unwrap_err().decode_fault(),
            expected
        );
    }
}

#[test]
fn unsupported_charset_errors() {
    for &cs in &[charset::IBM_MICROSOFT_DBCS, charset::JIS_X_0208] {
        let data = [cs, 0x41, 0x42];
        let result = decode_character_string(&data);
        assert!(result.is_err(), "charset {cs} should return an error");
    }
}
