#![no_main]

use bacnet_mstp_codec::{decode_extended_frame, decode_frame, MAX_EXTENDED_MPDU_DATA};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = decode_frame(input);
    let mut scratch = [0u8; MAX_EXTENDED_MPDU_DATA];
    let _ = decode_extended_frame(input, &mut scratch);
});
