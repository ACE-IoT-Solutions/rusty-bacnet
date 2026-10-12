//! One-shot decode of arbitrary octets.
//!
//! Oracle: `decode_frame` accepts exactly the inputs that start with a whole
//! valid standard frame by `mstp_codec_fuzz::valid_frame`, which checks the
//! layout and both CRCs without the codec, and reports that frame's length
//! and fields. A frame it accepts re-encodes to the same octets, unless the
//! encoder's stricter checks refuse it: data on a type defined without any,
//! or a proprietary frame too short for its vendor identifier.

#![no_main]

use bacnet_mstp_codec::{decode_frame, FrameError};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, HEADER_END};

fuzz_target!(
    init: wire::self_test(),
    |input: &[u8]| {
        let decoded = decode_frame(input);
        let Some(length) = wire::valid_frame(input) else {
            assert!(decoded.is_err(), "accepted an invalid frame: {decoded:?}");
            return;
        };
        let (frame, consumed) = decoded.expect("refused a valid frame");
        assert_eq!(consumed, length);
        let raw = input[2];
        assert_eq!(frame.header.frame_type.to_raw(), raw);
        assert_eq!(frame.header.destination, input[3]);
        assert_eq!(frame.header.source, input[4]);
        let data_length = frame.data.len();
        assert_eq!(wire::wire_length(data_length), length);
        assert_eq!(frame.data, &input[HEADER_END..HEADER_END + data_length]);

        let mut output = [0u8; wire::MAX_WIRE];
        match frame.encode_into(&mut output) {
            Ok(written) => assert_eq!(output[..written], input[..length]),
            Err(FrameError::UnexpectedData { raw_type, length }) => {
                assert!(wire::never_carries_data(raw_type) && length != 0)
            }
            Err(FrameError::MissingVendorId { raw_type, length }) => {
                assert!(wire::is_proprietary(raw_type) && length < 2)
            }
            Err(error) => panic!("a decoded frame failed to encode: {error:?}"),
        }
    }
);
