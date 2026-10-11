#![no_main]

use bacnet_mstp_codec::{
    decode_extended_frame, decode_frame, DecodeEvent, StreamDecoder, MAX_EXTENDED_FRAME_LENGTH,
    MAX_EXTENDED_MPDU_DATA,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = decode_frame(input);
    let mut scratch = [0u8; MAX_EXTENDED_MPDU_DATA];
    let _ = decode_extended_frame(input, &mut scratch);

    // Exercise the actual receive state machine as bytes arrive in arbitrary
    // DMA/serial-sized chunks.  The one-shot decoders above do not cover
    // retained preambles, partial headers, pending frames, or resynchronizing
    // malformed candidates.
    let mut storage = [0u8; MAX_EXTENDED_FRAME_LENGTH];
    let mut decoder = StreamDecoder::new(&mut storage);
    let mut offset = 0;
    while offset < input.len() {
        let chunk = (usize::from(input[offset]) % 32) + 1;
        let end = (offset + chunk).min(input.len());
        let event = decoder.push_slice(&input[offset..end]);
        offset = end;

        // A decoded frame remains pending until the next push.  Commit that
        // state during fuzzing so a single input can exercise concatenated
        // frames and the suffix retained behind a completed frame.
        if matches!(
            event,
            DecodeEvent::Frame { .. } | DecodeEvent::ExtendedFrame { .. }
        ) {
            let _ = decoder.push_slice(&[]);
        }
    }
});
