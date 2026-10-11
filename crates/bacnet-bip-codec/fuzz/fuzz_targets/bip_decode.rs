#![no_main]

use bacnet_bip_codec::{decode, BvllFrame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(frame) = BvllFrame::decode(input) {
        let _ = frame.message();
    }
    let _ = decode(input);
});
