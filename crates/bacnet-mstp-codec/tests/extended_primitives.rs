//! Extended MS/TP COBS/XOR-0x55 and CRC-32K primitive vectors.

use bacnet_mstp_codec::{
    cobs_decode_xor55, cobs_decode_xor55_in_place, cobs_encode_xor55, cobs_encoded_len, crc32k,
    crc32k_accumulate_all, crc32k_valid, CobsError, CRC32K_RESIDUAL,
};

#[test]
fn crc32k_addendum_vectors() {
    let input = [0x01, 0x22, 0x30];
    assert_eq!(crc32k_accumulate_all(&input), 0x83DD_5A41);
    assert_eq!(crc32k(&input), 0x7C22_A5BE);

    let mut with_crc = input.to_vec();
    with_crc.extend_from_slice(&[0xBE, 0xA5, 0x22, 0x7C]);
    assert!(crc32k_valid(&with_crc));
    assert_eq!(crc32k_accumulate_all(&with_crc), CRC32K_RESIDUAL);
    assert!(!crc32k_valid(&[0, 1, 2, 3]));
}

#[test]
fn independent_crc32k_check_and_embedded_xor_mask_vector() {
    // CRC-32K/Koopman's conventional ASCII check value, independent of the
    // BACnet Annex G byte sequence.
    assert_eq!(crc32k(b"123456789"), 0x2D3D_D0AE);

    // Ordinary COBS for [0x55, 0x00, 0x55] is [2, 0x55, 2, 0x55].
    // Applying the required XOR mask yields literal zero octets for the two
    // embedded 0x55 data bytes, which must still round-trip exactly.
    let input = [0x55, 0x00, 0x55];
    let expected = [0x57, 0x00, 0x57, 0x00];
    let mut encoded = [0xA5; 4];
    assert_eq!(cobs_encode_xor55(&input, &mut encoded), Ok(4));
    assert_eq!(encoded, expected);

    let mut decoded = [0xA5; 3];
    assert_eq!(cobs_decode_xor55(&encoded, &mut decoded), Ok(3));
    assert_eq!(decoded, input);
}

#[test]
fn annex_x_client_cobs_vector() {
    let client = b"Hello World\n\0";
    let expected = [
        0x58, 0x1D, 0x30, 0x39, 0x39, 0x3A, 0x75, 0x02, 0x3A, 0x27, 0x39, 0x31, 0x5F, 0x54,
    ];
    let mut encoded = [0u8; 14];
    assert_eq!(cobs_encode_xor55(client, &mut encoded), Ok(expected.len()));
    assert_eq!(encoded, expected);

    let mut decoded = [0u8; 13];
    assert_eq!(cobs_decode_xor55(&expected, &mut decoded), Ok(client.len()));
    assert_eq!(&decoded, client);

    // The Annex X CRC bytes are separately COBS/XOR-encoded into a five-byte
    // field.  The unmasked bytes are the complemented, little-endian CRC
    // shown by the normative example.
    let crc_field_input = [0x35, 0xD7, 0x70, 0x4C];
    let crc_field = [0x50, 0x60, 0x82, 0x25, 0x19];
    let mut encoded_crc = [0u8; 5];
    assert_eq!(cobs_encode_xor55(&crc_field_input, &mut encoded_crc), Ok(5));
    assert_eq!(encoded_crc, crc_field);
}

#[test]
fn cobs_254_boundaries_have_no_phantom_zero() {
    let input = [0xA5; 254];
    assert_eq!(cobs_encoded_len(&input), 255);
    let mut encoded = [0u8; 255];
    assert_eq!(cobs_encode_xor55(&input, &mut encoded), Ok(255));
    assert_eq!(encoded[0], 0xFF ^ 0x55);
    let mut decoded = [0u8; 254];
    assert_eq!(cobs_decode_xor55(&encoded, &mut decoded), Ok(254));
    assert_eq!(decoded, input);

    let input = [0xA5; 255];
    assert_eq!(cobs_encoded_len(&input), 257);
    let mut encoded = [0u8; 257];
    assert_eq!(cobs_encode_xor55(&input, &mut encoded), Ok(257));
    assert_eq!(encoded[0], 0xFF ^ 0x55);
    assert_eq!(encoded[255], 0x02 ^ 0x55);
    let mut decoded = [0u8; 255];
    assert_eq!(cobs_decode_xor55(&encoded, &mut decoded), Ok(255));
    assert_eq!(decoded, input);
}

#[test]
fn cobs_zero_boundaries_and_in_place_decode() {
    for input in [&[][..], &[0][..], &[1, 0][..], &[0, 1, 0, 2, 0][..]] {
        let mut encoded = [0xA5; 32];
        let used = cobs_encode_xor55(input, &mut encoded).unwrap();
        let mut decoded = [0xA5; 32];
        let decoded_len = cobs_decode_xor55(&encoded[..used], &mut decoded).unwrap();
        assert_eq!(&decoded[..decoded_len], input);

        let mut in_place = encoded;
        assert_eq!(
            cobs_decode_xor55_in_place(&mut in_place[..used]),
            Ok(input.len())
        );
        assert_eq!(&in_place[..input.len()], input);
    }
}

#[test]
fn malformed_and_exhausted_operations_are_atomic() {
    assert_eq!(
        cobs_decode_xor55(&[0x55], &mut [0u8; 1]),
        Err(CobsError::InvalidCode { offset: 0 })
    );
    assert_eq!(
        cobs_decode_xor55(&[0x57, 0x55], &mut [0u8; 1]),
        Err(CobsError::NonCanonicalZero { offset: 1 })
    );
    assert_eq!(
        cobs_decode_xor55(&[0x57], &mut [0u8; 1]),
        Err(CobsError::CodeExceedsInput {
            offset: 0,
            code: 2,
            available: 0
        })
    );

    let input = [1, 2, 3, 4];
    let mut output = [0xA5; 4];
    assert_eq!(
        cobs_encode_xor55(&input, &mut output),
        Err(CobsError::OutputExhausted {
            needed: 5,
            available: 4
        })
    );
    assert_eq!(output, [0xA5; 4]);

    let encoded = [0x04 ^ 0x55, 1 ^ 0x55, 2 ^ 0x55, 3 ^ 0x55];
    let mut output = [0xA5; 2];
    assert_eq!(
        cobs_decode_xor55(&encoded, &mut output),
        Err(CobsError::OutputExhausted {
            needed: 3,
            available: 2
        })
    );
    assert_eq!(output, [0xA5; 2]);
}
