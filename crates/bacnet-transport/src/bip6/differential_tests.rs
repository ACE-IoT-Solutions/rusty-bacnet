//! Differential test for #1621: the hand-written B/IPv6 codec in `frame.rs`
//! against `bacnet_bip_codec::ipv6`, over a corpus of valid and invalid
//! datagrams, before the old code is deleted.
//!
//! Every datagram must decode the same way in both, or differ in one of the
//! intended ways the cutover keeps behaviour for:
//!
//! - **Unknown functions** (0x0B, 0x0D and up): the old decoder returned a
//!   frame the receive loop then ignored; the codec refuses them.
//! - **Forwarded-NPDU without a whole address** (payload under 21 octets):
//!   the old decoder returned a frame whose payload
//!   `decode_forwarded_npdu_payload` then refused; the codec refuses it.
//! - **Forwarded-NPDU with an NPDU under 2 octets**: the old payload decoder
//!   refused it; the codec decodes it and the transport must drop it.
//!
//! The encoders must agree octet for octet.

use std::collections::BTreeMap;
use std::net::SocketAddrV6;

use bacnet_bip_codec::ipv6::{
    decode, Bip6Address, Bip6Vmac, Bvlc6Function as Codec6Function, Bvlc6ResultCode, Bvll6,
    Bvll6Error,
};
use bytes::BytesMut;

use super::*;

/// xorshift64*: a small deterministic generator, so the corpus is the same
/// on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.byte()).collect()
    }

    /// Between `least` and `least + spread - 1` random octets.
    fn some_bytes(&mut self, least: usize, spread: usize) -> Vec<u8> {
        let length = least + self.below(spread);
        self.bytes(length)
    }

    fn vmac(&mut self) -> Bip6Vmac {
        Bip6Vmac::new([self.byte(), self.byte(), self.byte()])
    }

    fn address(&mut self) -> Bip6Address {
        let mut octets = [0u8; 18];
        octets.iter_mut().for_each(|octet| *octet = self.byte());
        Bip6Address::from_octets(octets)
    }
}

/// A header with `function` and a matching Length field, then `payload`.
fn frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).unwrap().to_be_bytes();
    let mut wire = vec![0x82, function, length[0], length[1]];
    wire.extend_from_slice(payload);
    wire
}

/// What the old decoder's `Bvlc6Frame` held, rebuilt from a codec message's
/// fields: the function octet, the first VMAC, the second VMAC of the
/// functions the old decoder read one for, and the rest of the payload.
fn old_view(message: &Bvll6<'_>) -> (u8, Bip6Vmac, Option<Bip6Vmac>, Vec<u8>) {
    let function = message.function().to_raw();
    let source = message.source_vmac();
    let (second, rest): (Option<Bip6Vmac>, Vec<u8>) = match *message {
        Bvll6::BvlcResult { code, .. } => (None, code.to_raw().to_be_bytes().to_vec()),
        Bvll6::OriginalUnicastNpdu {
            destination, npdu, ..
        } => (Some(destination), npdu.to_vec()),
        Bvll6::OriginalBroadcastNpdu { npdu, .. }
        | Bvll6::DistributeBroadcastToNetwork { npdu, .. } => (None, npdu.to_vec()),
        Bvll6::AddressResolution { target, .. } => (Some(target), Vec::new()),
        Bvll6::ForwardedAddressResolution {
            target,
            original_address,
            ..
        } => (Some(target), original_address.to_octets().to_vec()),
        Bvll6::AddressResolutionAck { destination, .. }
        | Bvll6::VirtualAddressResolutionAck { destination, .. } => (Some(destination), Vec::new()),
        Bvll6::VirtualAddressResolution { .. } => (None, Vec::new()),
        Bvll6::ForwardedNpdu {
            original_address,
            npdu,
            ..
        } => {
            let mut rest = original_address.to_octets().to_vec();
            rest.extend_from_slice(npdu);
            (None, rest)
        }
        Bvll6::RegisterForeignDevice { ttl, .. } => (None, ttl.to_be_bytes().to_vec()),
        Bvll6::DeleteForeignDeviceTableEntry { entry, .. } => (None, entry.to_octets().to_vec()),
    };
    (function, source, second, rest)
}

/// Compare the two decoders on one datagram and name the outcome.
fn compare(data: &[u8]) -> &'static str {
    let old = decode_bvlc6(data);
    let new = decode(data);
    match (old, new) {
        (Err(_), Err(_)) => "both refuse",
        (Ok(old), Ok(new)) => {
            let (function, source, second, rest) = old_view(&new);
            assert_eq!(old.function.to_byte(), function, "{data:02X?}");
            assert_eq!(old.source_vmac, source.octets(), "{data:02X?}");
            assert_eq!(
                old.destination_vmac,
                second.map(Bip6Vmac::octets),
                "{data:02X?}"
            );
            assert_eq!(old.payload[..], rest[..], "{data:02X?}");
            let Bvll6::ForwardedNpdu {
                original_address,
                npdu,
                ..
            } = new
            else {
                return "both decode, equal";
            };
            match decode_forwarded_npdu_payload(&old.payload) {
                Ok((origin, old_npdu)) => {
                    assert_eq!(origin, SocketAddrV6::from(original_address));
                    assert_eq!(old_npdu, npdu);
                    "both decode, equal"
                }
                Err(_) => {
                    assert!(npdu.len() < 2, "{data:02X?}");
                    "forwarded NPDU under 2 octets: old payload refused, codec decodes"
                }
            }
        }
        (Ok(old), Err(Bvll6Error::UnknownFunction { function })) => {
            assert_eq!(old.function, Bvlc6Function::Unknown(function));
            "unknown function: old frame, codec refuses"
        }
        (Ok(old), Err(Bvll6Error::PayloadLength { function, length }))
            if function == Codec6Function::FORWARDED_NPDU =>
        {
            assert_eq!(old.function, Bvlc6Function::ForwardedNpdu);
            assert!(length < 21, "{data:02X?}");
            assert!(decode_forwarded_npdu_payload(&old.payload).is_err());
            "forwarded NPDU without a whole address: old payload refused, codec refuses"
        }
        (old, new) => panic!("unexpected difference on {data:02X?}: {old:?} vs {new:?}"),
    }
}

/// Valid messages of every kind, from the old encoders.
fn valid(rng: &mut Rng) -> Vec<u8> {
    let source = rng.vmac().octets();
    let second = rng.vmac().octets();
    let npdu = rng.some_bytes(0, 40);
    let address = rng.address().to_octets();
    let mut buf = BytesMut::new();
    match rng.below(12) {
        0 => encode_bvlc6_original_unicast(&mut buf, &source, &second, &npdu).unwrap(),
        1 => encode_bvlc6_original_broadcast(&mut buf, &source, &npdu).unwrap(),
        2 => buf = encode_address_resolution(&source, &second),
        3 => buf = encode_address_resolution_ack(&source, &second),
        4 => buf = encode_virtual_address_resolution(&source),
        5 => buf = encode_virtual_address_resolution_ack(&source, &second),
        6 => {
            let mut payload = address.to_vec();
            payload.extend_from_slice(&npdu);
            encode_bvlc6(&mut buf, Bvlc6Function::ForwardedNpdu, &source, &payload).unwrap();
        }
        7 => encode_bvlc6(
            &mut buf,
            Bvlc6Function::RegisterForeignDevice,
            &source,
            &[rng.byte(), rng.byte()],
        )
        .unwrap(),
        8 => encode_bvlc6(
            &mut buf,
            Bvlc6Function::DeleteForeignDeviceEntry,
            &source,
            &address,
        )
        .unwrap(),
        9 => encode_bvlc6(
            &mut buf,
            Bvlc6Function::DistributeBroadcastToNetwork,
            &source,
            &npdu,
        )
        .unwrap(),
        10 => encode_bvlc6(
            &mut buf,
            Bvlc6Function::Result,
            &source,
            &[0x00, [0x00, 0x30, 0x60, 0x90, 0xA0, 0xC0][rng.below(6)]],
        )
        .unwrap(),
        _ => {
            let mut payload = second.to_vec();
            payload.extend_from_slice(&address);
            encode_bvlc6(
                &mut buf,
                Bvlc6Function::ForwardedAddressResolution,
                &source,
                &payload,
            )
            .unwrap();
        }
    }
    buf.to_vec()
}

/// A valid message with its header or length damaged.
fn mutated(rng: &mut Rng) -> Vec<u8> {
    let mut wire = valid(rng);
    match rng.below(6) {
        0 => wire[0] = rng.byte(),
        1 => {
            let declared = rng.below(wire.len() + 8) as u16;
            wire[2..4].copy_from_slice(&declared.to_be_bytes());
        }
        2 => wire.truncate(rng.below(wire.len())),
        3 => wire.extend(rng.some_bytes(1, 4)),
        4 => {
            // Extended, with the Length field following it.
            wire.extend(rng.some_bytes(1, 4));
            let length = u16::try_from(wire.len()).unwrap();
            wire[2..4].copy_from_slice(&length.to_be_bytes());
        }
        _ => {
            // Another function's code on this payload.
            wire[1] = rng.below(0x11) as u8;
        }
    }
    wire
}

#[test]
#[allow(clippy::print_stderr)]
fn old_and_new_decoders_agree_on_the_corpus() {
    let mut rng = Rng(0x1621_B1B6_C0DE_C0DE);
    let mut outcomes: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut tally = |data: &[u8]| *outcomes.entry(compare(data)).or_default() += 1;

    // Every function code worth trying, every payload length up to 48: zeros,
    // ones and random octets.
    let functions: Vec<u8> = (0x00..=0x10).chain([0x7F, 0x80, 0xFF]).collect();
    for &function in &functions {
        for length in 0..=48 {
            tally(&frame(function, &vec![0x00; length]));
            tally(&frame(function, &vec![0xFF; length]));
            for _ in 0..16 {
                tally(&frame(function, &rng.bytes(length)));
            }
        }
    }
    for _ in 0..60_000 {
        let wire = valid(&mut rng);
        tally(&wire);
    }
    for _ in 0..60_000 {
        let wire = mutated(&mut rng);
        tally(&wire);
    }
    for _ in 0..40_000 {
        let length = rng.below(64);
        let mut wire = rng.bytes(length);
        if length > 0 && rng.below(2) == 0 {
            wire[0] = 0x82;
        }
        tally(&wire);
    }
    // Short datagrams, every type and function with every short length.
    for length in 0..7 {
        for function in [0x00, 0x01, 0x06, 0x08, 0x0B, 0xFF] {
            tally(&frame(function, &vec![0; length]));
            let mut short = frame(function, &[0; 3]);
            short.truncate(length);
            tally(&short);
        }
    }

    let total: usize = outcomes.values().sum();
    eprintln!("{total} datagrams:");
    for (outcome, count) in &outcomes {
        eprintln!("  {count:>7}  {outcome}");
    }
    assert!(outcomes["both decode, equal"] > 50_000);
    assert!(outcomes["both refuse"] > 10_000);
}

#[test]
#[allow(clippy::print_stderr)]
fn old_and_new_encoders_agree() {
    let mut rng = Rng(0x0BAC_0BAC_0BAC_0BAC);
    let mut cases = 0;
    let mut check = |old: &[u8], new: &Bvll6<'_>| {
        let mut wire = vec![0u8; new.encoded_len().unwrap()];
        new.encode_into(&mut wire).unwrap();
        assert_eq!(old, &wire[..], "{new:?}");
        cases += 1;
    };
    for _ in 0..2_000 {
        let source = rng.vmac();
        let second = rng.vmac();
        let npdu = rng.some_bytes(0, 64);
        let address = rng.address();
        let ttl = u16::from_be_bytes([rng.byte(), rng.byte()]);
        let code = [0x0000u16, 0x30, 0x60, 0x90, 0xA0, 0xC0][rng.below(6)];
        let (s, d) = (source.octets(), second.octets());

        let mut old = BytesMut::new();
        encode_bvlc6_original_unicast(&mut old, &s, &d, &npdu).unwrap();
        check(
            &old,
            &Bvll6::OriginalUnicastNpdu {
                source,
                destination: second,
                npdu: &npdu,
            },
        );
        let mut old = BytesMut::new();
        encode_bvlc6_original_broadcast(&mut old, &s, &npdu).unwrap();
        check(
            &old,
            &Bvll6::OriginalBroadcastNpdu {
                source,
                npdu: &npdu,
            },
        );
        let mut old = BytesMut::new();
        encode_bvlc6(
            &mut old,
            Bvlc6Function::DistributeBroadcastToNetwork,
            &s,
            &npdu,
        )
        .unwrap();
        check(
            &old,
            &Bvll6::DistributeBroadcastToNetwork {
                original_source: source,
                npdu: &npdu,
            },
        );
        check(
            &encode_address_resolution(&s, &d),
            &Bvll6::AddressResolution {
                source,
                target: second,
            },
        );
        check(
            &encode_address_resolution_ack(&s, &d),
            &Bvll6::AddressResolutionAck {
                source,
                destination: second,
            },
        );
        check(
            &encode_virtual_address_resolution(&s),
            &Bvll6::VirtualAddressResolution { source },
        );
        check(
            &encode_virtual_address_resolution_ack(&s, &d),
            &Bvll6::VirtualAddressResolutionAck {
                source,
                destination: second,
            },
        );
        let mut payload = address.to_octets().to_vec();
        payload.extend_from_slice(&npdu);
        let mut old = BytesMut::new();
        encode_bvlc6(&mut old, Bvlc6Function::ForwardedNpdu, &s, &payload).unwrap();
        check(
            &old,
            &Bvll6::ForwardedNpdu {
                original_source: source,
                original_address: address,
                npdu: &npdu,
            },
        );
        let mut old = BytesMut::new();
        encode_bvlc6(
            &mut old,
            Bvlc6Function::RegisterForeignDevice,
            &s,
            &ttl.to_be_bytes(),
        )
        .unwrap();
        check(&old, &Bvll6::RegisterForeignDevice { source, ttl });
        let mut old = BytesMut::new();
        encode_bvlc6(
            &mut old,
            Bvlc6Function::DeleteForeignDeviceEntry,
            &s,
            &address.to_octets(),
        )
        .unwrap();
        check(
            &old,
            &Bvll6::DeleteForeignDeviceTableEntry {
                source,
                entry: address,
            },
        );
        let mut old = BytesMut::new();
        encode_bvlc6(&mut old, Bvlc6Function::Result, &s, &code.to_be_bytes()).unwrap();
        check(
            &old,
            &Bvll6::BvlcResult {
                source,
                code: Bvlc6ResultCode::from_raw(code),
            },
        );
    }
    eprintln!("{cases} encodings identical");

    // Both refuse the same oversize messages, and take the largest.
    let s = [1, 2, 3];
    let npdu = vec![0u8; 65_535];
    for length in [65_524, 65_525, 65_526] {
        let mut old = BytesMut::new();
        let old = encode_bvlc6_original_unicast(&mut old, &s, &s, &npdu[..length]).map(|_| old);
        let new = Bvll6::OriginalUnicastNpdu {
            source: Bip6Vmac::new(s),
            destination: Bip6Vmac::new(s),
            npdu: &npdu[..length],
        };
        let mut wire = vec![0u8; 65_540];
        match (old, new.encode_into(&mut wire)) {
            (Ok(old), Ok(written)) => assert_eq!(old[..], wire[..written]),
            (Err(_), Err(Bvll6Error::TooLong { .. })) => {}
            (old, new) => panic!("{length}: {old:?} vs {new:?}"),
        }
    }
    for length in [65_527, 65_528, 65_529] {
        let mut old = BytesMut::new();
        let old = encode_bvlc6_original_broadcast(&mut old, &s, &npdu[..length]).map(|_| old);
        let new = Bvll6::OriginalBroadcastNpdu {
            source: Bip6Vmac::new(s),
            npdu: &npdu[..length],
        };
        let mut wire = vec![0u8; 65_540];
        match (old, new.encode_into(&mut wire)) {
            (Ok(old), Ok(written)) => assert_eq!(old[..], wire[..written]),
            (Err(_), Err(Bvll6Error::TooLong { .. })) => {}
            (old, new) => panic!("{length}: {old:?} vs {new:?}"),
        }
    }
}
