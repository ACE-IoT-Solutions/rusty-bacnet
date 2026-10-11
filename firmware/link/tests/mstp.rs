//! Runs the MS/TP image's scenario natively and checks what it reports.
//! This is where the scenario's behaviour is verified: the bare-metal build
//! only links it.

use core::mem::{size_of, MaybeUninit};

use bacnet_mstp_codec::{FrameType, StreamDecoder, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::MasterState;
use firmware_link::mstp::{run, Core, Memory, Queues, Summary};

/// The peer station the scenario exchanges the token with.
const PEER: u8 = 2;

#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    let mut core = MaybeUninit::uninit();
    let summary = run(&mut memory, &mut core);
    let expected = Summary {
        // The five frames of the codec part (a Who-Is among them) go through
        // the stream decoder; the encoder refuses a Token with data.
        frames_encoded: 5,
        encode_refusals: 1,
        frames_decoded: 5,
        // The Who-Is copy with a bad data CRC, and two octets of noise.
        frames_malformed: 1,
        noise_octets: 2,
        // Transmit and back to receive for each of the four frames sent.
        direction_changes: 8,
        // With the token: the queued Who-Is to the peer, then a poll for the
        // next master, which is the peer. The peer answers, so the token
        // goes to it. Then the answer to the peer's request.
        sent: [
            Some((FrameType::BACnetDataNotExpectingReply, PEER)),
            Some((FrameType::PollForMaster, PEER)),
            Some((FrameType::Token, PEER)),
            Some((FrameType::BACnetDataNotExpectingReply, PEER)),
            None,
            None,
            None,
            None,
        ],
        // The peer's broadcast and its request.
        npdus_delivered: 2,
        requests: 1,
        state: MasterState::Idle,
        // The token, the peer's answer to the poll, its broadcast and its
        // request.
        frames_received: 4,
        frames_transmitted: 4,
        token_rotations: 1,
        parser_errors: 0,
        // Every unicast slot fills, the next NPDU is refused, and so is one
        // a standard frame can't carry.
        queued_before_full: 4,
        drops_full: 1,
        drops_oversize: 1,
        errors: 0,
    };
    assert_eq!(summary, expected);
}

/// Prints the sizes behind the image's RAM use, for the host's pointer
/// width; check-firmware-link.sh prints the bare-metal ones from the image.
/// nextest shows the output (.config/nextest.toml).
#[test]
fn ram_sizes() {
    let core = size_of::<Core<'static>>();
    let queues = size_of::<Queues>();
    let decoder = size_of::<StreamDecoder<'static>>();
    println!("MasterCore<HeaplessQueues<4, 2>>: {core} octets, its queues {queues}");
    println!(
        "StreamDecoder: {decoder} octets, {} with {MAX_STANDARD_FRAME_LENGTH} octets of storage",
        decoder + MAX_STANDARD_FRAME_LENGTH
    );
    println!("Memory: {} octets", size_of::<Memory>());
    // The core holds its queues and its decoder, not the decoder's storage.
    assert!(core > queues + decoder);
}
