//! Runs the B/IP image's scenario natively and checks what it reports.
//! This is where the scenario's behaviour is verified: the bare-metal build
//! only links it.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntry, BvlcFunction, BvlcResultCode, BvllError, FdtEntry, PayloadShape};
use firmware_link::bip::{run, Memory, Summary};

fn address(octets: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(octets), port)
}

#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    let summary = run(&mut memory);
    let expected = Summary {
        // One message of each of the twelve variants, through a slice and
        // through the sink, each decoding back to itself.
        encoded: 12,
        // Header-only reads 4 octets each, the result and the registration
        // 6, the deletion 10, the three tables 24, the Forwarded-NPDU 18 and
        // the three other NPDU carriers 12.
        encoded_octets: 4 * 2 + 6 * 2 + 10 + 24 * 3 + 18 + 12 * 3,
        sink_matches: 12,
        round_trips: 12,
        // Function codes 0x00 through 0x0B.
        functions: 0x0FFF,
        bdt_ends: [
            Some(BdtEntry::new(
                address([192, 168, 1, 10], 47808),
                Ipv4Addr::new(255, 255, 255, 255),
            )),
            Some(BdtEntry::new(
                address([10, 0, 0, 1], 47808),
                Ipv4Addr::new(255, 255, 255, 0),
            )),
        ],
        fdt_ends: [
            Some(FdtEntry::new(address([192, 168, 2, 20], 47809), 300, 312)),
            Some(FdtEntry::new(address([172, 16, 0, 5], 47808), 60, 4)),
        ],
        origin: Some(address([10, 0, 0, 77], 47808)),
        forwarded_npdu: Some(8),
        shapes: [
            Some(PayloadShape::Fixed(2)),
            Some(PayloadShape::Table { entry_length: 10 }),
            Some(PayloadShape::Npdu { prefix: 6 }),
            None,
        ],
        naks: [
            Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK),
            Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK),
            None,
        ],
        rejections: [
            Some(BvllError::LengthMismatch {
                declared: 6,
                received: 7,
            }),
            Some(BvllError::PayloadLength {
                function: BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                length: 9,
            }),
            Some(BvllError::TableLength {
                length: 9,
                entry_length: 10,
            }),
            Some(BvllError::UndefinedResultCode { code: 0x0070 }),
            Some(BvllError::UnknownFunction { function: 0x0C }),
        ],
        errors: 0,
    };
    assert_eq!(summary, expected);
}
