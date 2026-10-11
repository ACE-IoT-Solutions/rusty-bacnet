//! Differential test: this module's BVLL decoder and encoders against
//! `bacnet-bip-codec`, before the transport moves onto the codec.
//!
//! Every datagram of a corpus (golden messages, a systematic sweep of
//! functions, payload lengths and Length fields, and pseudo-random octets)
//! goes through both decoders. They must agree, except in the categories the
//! cutover changes on purpose, which are counted and printed:
//!
//! - trailing octets after the declared length, which this decoder trims and
//!   the codec refuses;
//! - a payload of the wrong length for a defined management function, which
//!   this decoder hands on raw and the codec refuses at its message step
//!   (the frame step still accepts it, so the transport can still answer);
//! - an unknown function, which both accept as a frame, while the codec has
//!   no typed message for it.

use bacnet_bip_codec::{self as codec, BvllError, BvllFrame};
use bytes::BytesMut;

use super::{decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, encode_bvll_forwarded};
use bacnet_types::enums::BvlcFunction as OldFunction;

/// Deterministic xorshift, so the corpus is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

fn header(bvlc_type: u8, function: u8, declared: u16) -> Vec<u8> {
    let [high, low] = declared.to_be_bytes();
    vec![bvlc_type, function, high, low]
}

fn corpus() -> Vec<Vec<u8>> {
    let mut corpus = Vec::new();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // Shorter than a header.
    for length in 0..4 {
        corpus.push(vec![0x81, 0x0A, 0x00, 0x04][..length].to_vec());
    }
    // Every function 0x00-0x10, payload lengths 0-32, and Length fields
    // exact, one short, one long, below the header, and zero.
    for bvlc_type in [0x81, 0x82, 0x00] {
        for function in 0x00..=0x10u8 {
            for payload_length in 0..=32usize {
                let payload: Vec<u8> = (0..payload_length).map(|_| rng.byte()).collect();
                let exact = 4 + payload_length;
                for declared in [exact, exact - 1, exact + 1, 3, 2, 0, 4] {
                    let mut wire = header(bvlc_type, function, declared as u16);
                    wire.extend_from_slice(&payload);
                    corpus.push(wire);
                }
            }
        }
    }
    // Long NPDUs and tables near the limits that matter.
    for function in [0x03, 0x04, 0x07, 0x0A, 0x0B] {
        for payload_length in [1490, 1491, 1497, 1500, 1501] {
            let mut wire = header(0x81, function, (4 + payload_length) as u16);
            wire.extend((0..payload_length).map(|_| rng.byte()));
            corpus.push(wire);
        }
    }
    // Pseudo-random datagrams, most with a plausible header.
    for _ in 0..200_000 {
        let length = (rng.next() % 40) as usize;
        let mut wire: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
        if length >= 4 && !rng.next().is_multiple_of(4) {
            wire[0] = 0x81;
            wire[1] %= 0x0E;
            let skew = (rng.next() % 5) as usize;
            let declared = match rng.next() % 4 {
                0 => length + skew,
                1 => length - skew.min(length),
                _ => length,
            };
            wire[2..4].copy_from_slice(&(declared as u16).to_be_bytes());
        }
        corpus.push(wire);
    }
    corpus
}

#[derive(Debug, Default)]
struct Tally {
    identical: usize,
    trailing_octets: usize,
    malformed_payload: usize,
    unknown_function: usize,
}

#[test]
fn old_and_new_decoders_agree_except_for_intended_changes() {
    let mut tally = Tally::default();
    for wire in corpus() {
        let old = decode_bvll(&wire);
        let frame = BvllFrame::decode(&wire);
        match frame {
            Ok(frame) => match (old, frame.message()) {
                (Ok(old), Ok(message)) => {
                    assert_eq!(old.function.to_raw(), frame.function.to_raw());
                    assert_eq!(
                        old.payload,
                        message.npdu().unwrap_or(frame.payload),
                        "{wire:02X?}"
                    );
                    let origin = message
                        .origin()
                        .map(|origin| (origin.ip().octets(), origin.port()));
                    assert_eq!(old.originating_ip, origin.map(|(ip, _)| ip));
                    assert_eq!(old.originating_port, origin.map(|(_, port)| port));
                    tally.identical += 1;
                }
                // A Forwarded-NPDU too short for its origin: both refuse.
                (Err(_), Err(BvllError::PayloadLength { function, .. }))
                    if function == codec::BvlcFunction::FORWARDED_NPDU =>
                {
                    tally.identical += 1;
                }
                (Ok(old), Err(BvllError::UnknownFunction { function })) => {
                    assert_eq!(old.payload, frame.payload);
                    assert_eq!(function, old.function.to_raw());
                    tally.unknown_function += 1;
                }
                (Ok(old), Err(BvllError::PayloadLength { function, .. })) => {
                    // The old decoder never read a management payload.
                    assert_ne!(function, codec::BvlcFunction::FORWARDED_NPDU);
                    assert_eq!(old.function.to_raw(), function.to_raw());
                    assert_eq!(old.payload, frame.payload);
                    tally.malformed_payload += 1;
                }
                (old, new) => panic!("{wire:02X?}: old {old:?}, new {new:?}"),
            },
            Err(BvllError::LengthMismatch { declared, received })
                if declared >= 4 && declared < received =>
            {
                // The old decoder read the datagram as if it ended at the
                // declared length; the codec refuses the whole datagram.
                let trimmed = decode_bvll(&wire[..declared]).ok();
                assert_eq!(old.ok(), trimmed, "{wire:02X?}");
                if trimmed.is_some() {
                    tally.trailing_octets += 1;
                } else {
                    tally.identical += 1;
                }
            }
            Err(error) => {
                assert!(
                    old.is_err(),
                    "{wire:02X?}: new refused ({error}), old accepted"
                );
                tally.identical += 1;
            }
        }
    }
    println!("{tally:?}");
    assert!(tally.trailing_octets > 0 && tally.malformed_payload > 0);
    assert!(tally.unknown_function > 0 && tally.identical > 0);
}

#[test]
fn old_and_new_encoders_write_the_same_octets() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for _ in 0..2_000 {
        let npdu: Vec<u8> = (0..(rng.next() % 64) as usize)
            .map(|_| rng.byte())
            .collect();
        let ip = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
        let port = rng.next() as u16;
        let origin = codec::address_from_octets(encode_bip_mac(ip, port));
        assert_eq!(
            decode_bip_mac(&codec::address_to_octets(origin)).unwrap(),
            (ip, port)
        );

        let cases = [
            (
                OldFunction::ORIGINAL_UNICAST_NPDU,
                codec::Bvll::OriginalUnicastNpdu(&npdu),
            ),
            (
                OldFunction::ORIGINAL_BROADCAST_NPDU,
                codec::Bvll::OriginalBroadcastNpdu(&npdu),
            ),
            (
                OldFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                codec::Bvll::DistributeBroadcastToNetwork(&npdu),
            ),
        ];
        for (function, message) in cases {
            let mut old = BytesMut::new();
            encode_bvll(&mut old, function, &npdu).unwrap();
            let mut new = BytesMut::new();
            message.encode_to_sink(&mut new).unwrap();
            assert_eq!(old, new);
        }
        let mut old = BytesMut::new();
        encode_bvll_forwarded(&mut old, ip, port, &npdu).unwrap();
        let mut new = BytesMut::new();
        codec::Bvll::ForwardedNpdu {
            origin,
            npdu: &npdu,
        }
        .encode_to_sink(&mut new)
        .unwrap();
        assert_eq!(old, new);

        let ttl = rng.next() as u16;
        let mut old = BytesMut::new();
        encode_bvll(
            &mut old,
            OldFunction::REGISTER_FOREIGN_DEVICE,
            &ttl.to_be_bytes(),
        )
        .unwrap();
        let mut new = BytesMut::new();
        codec::Bvll::RegisterForeignDevice { ttl }
            .encode_to_sink(&mut new)
            .unwrap();
        assert_eq!(old, new);
    }
}
