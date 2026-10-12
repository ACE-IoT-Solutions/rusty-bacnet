//! Encode an arbitrary frame through every sink and decode what each wrote.
//!
//! Oracle: the outcome the crate docs give, worked out here without the
//! codec. A frame the encode checks refuse (a COBS type, more than 501
//! octets of data, source 255, data on a type defined without any, a
//! proprietary frame under two octets, in that order) gets that error from
//! every sink, and no sink changes. A frame that passes is the octets
//! `mstp_codec_fuzz::encode` builds from the layout, in every sink with room
//! for it; a fixed sink without room refuses it with `BufferExhausted` and
//! keeps its contents, which the target compares before and after. Each
//! successful encode decodes back to the frame, ignoring octets after it.

#![no_main]

use arbitrary::Arbitrary;
use bacnet_mstp_codec::{decode_frame, Frame, FrameError, FrameSink, FrameType};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, SinkKnobs, MAX_DATA};

#[derive(Debug, Arbitrary)]
struct Input {
    frame_type: u8,
    /// Build the type as `FrameType::Unknown(raw)` even for 0-7, which the
    /// codec must treat by its raw value.
    hand_built_type: bool,
    destination: u8,
    source: u8,
    /// The data length, modulo 640: past the 501-octet limit too.
    length: u16,
    /// Repeated to fill the data field.
    pattern: Vec<u8>,
    sinks: SinkKnobs,
    /// Octets after the frame when it is decoded again.
    trailing: Vec<u8>,
}

/// The error the encode checks give, in the codec's order, or `None` when
/// the frame passes them.
fn refusal(raw: u8, source: u8, length: usize) -> Option<FrameError> {
    Some(if wire::is_cobs(raw) {
        FrameError::UnsupportedCobsFrame { raw_type: raw }
    } else if length > MAX_DATA {
        FrameError::Oversize {
            length,
            maximum: MAX_DATA,
        }
    } else if source == 0xFF {
        FrameError::BroadcastSource
    } else if length != 0 && wire::never_carries_data(raw) {
        FrameError::UnexpectedData {
            raw_type: raw,
            length,
        }
    } else if wire::is_proprietary(raw) && length < 2 {
        FrameError::MissingVendorId {
            raw_type: raw,
            length,
        }
    } else {
        return None;
    })
}

fuzz_target!(
    init: wire::self_test(),
    |input: Input| {
        let raw = input.frame_type;
        let data = wire::fill(usize::from(input.length) % 640, &input.pattern);
        let frame_type = if input.hand_built_type {
            FrameType::Unknown(raw)
        } else {
            FrameType::from_raw(raw)
        };
        let frame = Frame::new(frame_type, input.destination, input.source, &data);
        let wire_length = wire::wire_length(data.len());
        let expected = match refusal(raw, input.source, data.len()) {
            Some(error) => Err(error),
            None => Ok(wire::encode(raw, input.destination, input.source, &data)),
        };

        let outputs = wire::check_sinks(
            &expected,
            wire_length,
            &input.sinks,
            |needed, available| FrameError::BufferExhausted { needed, available },
            |output| frame.encode_into(output),
            |sink: &mut dyn FrameSink| frame.encode_to_sink(sink),
        );

        for mut output in outputs {
            let length = output.len();
            output.extend_from_slice(&input.trailing);
            let (decoded, consumed) = decode_frame(&output).expect("an encoded frame decodes");
            assert_eq!(consumed, length);
            assert_eq!(decoded.header.frame_type.to_raw(), raw);
            assert_eq!(decoded.header.frame_type, FrameType::from_raw(raw));
            assert_eq!(decoded.header.destination, input.destination);
            assert_eq!(decoded.header.source, input.source);
            assert_eq!(decoded.data, data);
            if !input.hand_built_type {
                assert_eq!(decoded, frame);
            }
        }
    }
);
