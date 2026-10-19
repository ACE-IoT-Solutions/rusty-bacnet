//! The Devices table and the Who-Has panel fitted to their content
//! (#1892): routed devices in the target syntax with the router in a Via
//! column when it fits, columns sized to their cells rather than spread
//! across a wide terminal, and found objects with their names and the
//! browser's object labels. Frames at 80x24, 120x40 and 200x50.

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::core::target::{parse_target, Target};
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};
use crate::tui::worker::DeviceFeed;

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 50)];

/// The B/IP router the routed devices answer through.
const ROUTER: &str = "192.168.100.254:47808";

/// A device as the client's discovery table holds it, behind a router at
/// [`ROUTER`] when `routed` names its network and station.
fn discovered(instance: u32, vendor_id: u16, routed: Option<(u16, &[u8])>) -> DiscoveredDevice {
    let link = match routed {
        Some(_) => [192, 168, 100, 254, 0xBA, 0xC0],
        None => [192, 168, 100, (instance % 200) as u8, 0xBA, 0xC0],
    };
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(&link),
        max_apdu_length: if routed.is_some() { 480 } else { 1476 },
        segmentation_supported: Segmentation::BOTH,
        max_segments_accepted: None,
        vendor_id,
        last_seen: at(3),
        source_network: routed.map(|(network, _)| network),
        source_address: routed.map(|(_, station)| MacAddr::from_slice(station)),
    }
}

/// A local device, a routed B/IP device and a routed MS/TP station, the
/// rows written by the worker's own feed; the largest instance and vendor
/// ID there are.
fn site() -> App {
    let feed = DeviceFeed::new(AddressStyle::Bip);
    let mut app = connected_app();
    let devices = [
        discovered(1001, 260, None),
        discovered(4_194_302, 65_535, None),
        discovered(2001, 24, Some((2, &[0x7F, 0, 0, 1, 0xC3, 0x35]))),
        discovered(3005, 8, Some((3, &[5]))),
    ];
    for device in &devices {
        worker(&mut app, WorkerEvent::Discovered(feed.row(device)));
    }
    tick(&mut app, at(5));
    app
}

/// The table's header row: the first line with `Instance`.
fn header(frame: &str) -> &str {
    frame
        .lines()
        .find(|line| line.contains("Instance"))
        .unwrap_or_else(|| panic!("no header in\n{frame}"))
}

#[test]
fn routed_devices_show_their_station_as_a_target_that_reads_back() {
    let mut app = site();
    app.settle();
    let routed: Vec<_> = app
        .devices
        .visible()
        .filter(|row| row.via.is_some())
        .collect();
    let shown: Vec<_> = routed.iter().map(|row| row.address.as_str()).collect();
    assert_eq!(shown, ["2:0x7F000001C335", "3:0x05"]);
    let targets: Vec<_> = shown.iter().map(|text| parse_target(text)).collect();
    assert_eq!(
        targets,
        [
            Ok(Target::RoutedMac {
                dnet: 2,
                mac: vec![0x7F, 0, 0, 1, 0xC3, 0x35]
            }),
            Ok(Target::RoutedMac {
                dnet: 3,
                mac: vec![5]
            }),
        ]
    );
    assert!(routed.iter().all(|row| row.via.as_deref() == Some(ROUTER)));
}

#[test]
fn devices_are_never_cut_and_the_columns_fit_their_content() {
    for (width, height) in SIZES {
        let mut app = site();
        let frame = screen(&mut app, width, height);
        assert_snapshot!(format!("devices_site_{width}x{height}"), &frame);
        for row in app.devices.visible() {
            let line = frame
                .lines()
                .find(|line| line.contains(&format!(" {} ", row.instance)))
                .unwrap_or_else(|| panic!("instance {} cut at {width}:\n{frame}", row.instance));
            assert!(line.contains(&row.address), "{width}: {line}");
        }
        // The Via column needs the whole router address; at 80 it doesn't fit.
        let via = header(&frame).contains("Via");
        assert_eq!(via, width >= 120, "{width}:\n{frame}");
        assert_eq!(frame.contains(ROUTER), via, "{width}");
        // Content-sized: nothing pushes the last column to the far right.
        let header = header(&frame);
        let last = header.trim_end_matches(['"', '│', ' ']).chars().count();
        assert!(last < 110, "{width}: the header runs to {last}:\n{header}");
    }
}

#[test]
fn a_shorter_address_leaves_room_for_the_router_at_80_columns() {
    let mut app = connected_app();
    let routed = routed_row(2001, (2, &[5]), "10.0.1.1:47808", 24, 1);
    worker(&mut app, WorkerEvent::Discovered(routed));
    tick(&mut app, at(2));
    let frame = screen(&mut app, 80, 24);
    assert!(header(&frame).contains("Via"), "{frame}");
    assert!(frame.contains("2:0x05") && frame.contains("10.0.1.1:47808"));
}

/// The longest addresses there are, a B/IP one and a routed SC station on
/// the highest network, stay whole at 80 columns beside the widest instance.
#[test]
fn the_longest_addresses_are_whole_at_80_columns() {
    let mut app = connected_app();
    let local = row(4_194_303, "255.255.255.255:65535", 65_535, 1);
    let sc = (65_534, &[0xAB, 0xCD, 0xEF, 1, 2, 3][..]);
    let routed = routed_row(4_194_302, sc, "255.255.255.254:65535", 65_535, 1);
    worker(&mut app, WorkerEvent::Discovered(local));
    worker(&mut app, WorkerEvent::Discovered(routed));
    tick(&mut app, at(2));
    let frame = screen(&mut app, 80, 24);
    for (instance, address) in [
        ("4194303", "255.255.255.255:65535"),
        ("4194302", "65534:0xABCDEF010203"),
    ] {
        let line = frame.lines().find(|line| line.contains(instance));
        assert!(
            line.is_some_and(|line| line.contains(address) && line.contains("65535")),
            "{instance}:\n{frame}"
        );
    }
    assert!(!header(&frame).contains("Via"), "no room for it:\n{frame}");
}

/// A routed device that moves to another router is filtered again: the
/// filter matches the router, so the new one must be checked.
#[test]
fn a_new_router_is_filtered_again() {
    let mut app = connected_app();
    let first = routed_row(2001, (2, &[5]), "10.0.1.1:47808", 24, 1);
    worker(&mut app, WorkerEvent::Discovered(first));
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "10.0.9.");
    assert!(visible(&mut app).is_empty());
    let moved = routed_row(2001, (2, &[5]), "10.0.9.9:47808", 24, 2);
    worker(&mut app, WorkerEvent::Updated(moved));
    assert_eq!(visible(&mut app), [2001]);
}

/// Two routers reporting the same station for one instance: the banner
/// names the station through each router, not the station twice.
#[test]
fn a_routed_duplicate_names_both_routers() {
    let mut app = connected_app();
    let retained = routed_row(2001, (2, &[5]), "10.0.1.1:47808", 24, 1);
    let incoming = routed_row(2001, (2, &[5]), "10.0.1.2:47808", 24, 2);
    worker(&mut app, WorkerEvent::Collision { retained, incoming });
    let banner = app.duplicates.banner().join(" ");
    for router in ["10.0.1.1:47808", "10.0.1.2:47808"] {
        assert!(banner.contains(&format!("2:0x05 via {router}")), "{banner}");
    }
}

#[test]
fn the_sort_marker_widens_its_column_rather_than_crowd_it() {
    let mut app = site();
    // Sorted by address, descending.
    press(&mut app, KeyCode::Char('s'));
    let frame = screen(&mut app, 80, 24);
    let header = header(&frame);
    assert!(header.contains("Address*"), "{header}");
    assert!(header.contains("Instance "), "{header}");
}

/// The filter matches the router's address too.
#[test]
fn the_filter_finds_routed_devices_by_their_router() {
    let mut app = site();
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, ".254:");
    assert_eq!(visible(&mut app), [2001, 3005]);
}

const LONG_NAME: &str = "Building 4 East Wing AHU-07 Supply Air Temp Setpoint";

fn found_rows() -> [FoundRow; 2] {
    let object = |kind, instance| ObjectIdentifier::new(kind, instance).unwrap();
    [
        FoundRow {
            device: 1001,
            object: object(ObjectType::ANALOG_OUTPUT, 1),
            name: LONG_NAME.into(),
            address: "192.168.100.1:47808".into(),
            via: None,
            network: None,
        },
        FoundRow {
            device: 3005,
            object: object(ObjectType::ANALOG_VALUE, 4_194_302),
            name: LONG_NAME.into(),
            address: "3:0x05".into(),
            via: Some(ROUTER.into()),
            network: Some(3),
        },
    ]
}

/// The site, after `:find LONG_NAME` and both I-Haves for it.
fn who_has_app() -> App {
    let mut app = site();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, &format!("find {LONG_NAME}"));
    press(&mut app, KeyCode::Enter);
    for row in found_rows() {
        worker(&mut app, WorkerEvent::Discovery(DiscoveryEvent::IHave(row)));
    }
    assert_eq!(app.who_has.as_ref().unwrap().found.len(), 2);
    app
}

/// The results panel: from its title to the end of the frame.
fn panel(frame: &str) -> &str {
    &frame[frame.find("Who-Has").unwrap()..]
}

#[test]
fn who_has_shows_the_name_and_the_object_label() {
    for (width, height) in SIZES {
        let mut app = who_has_app();
        let frame = screen(&mut app, width, height);
        assert_snapshot!(format!("who_has_{width}x{height}"), &frame);
        let panel = panel(&frame);
        assert!(panel.contains("analog-output 1 "), "{panel}");
        assert!(panel.contains("analog-value 4194302"), "{panel}");
        assert!(!panel.contains("ANALOG"), "{panel}");
        assert!(panel.contains("3:0x05"), "{panel}");
        // The whole name fits from 120 columns; at 80 it ends in `…`.
        let rows = panel.lines().skip(2);
        let named = rows.clone().filter(|line| line.contains(LONG_NAME)).count();
        assert_eq!(named, if width >= 120 { 2 } else { 0 }, "{width}:\n{panel}");
        if width < 120 {
            assert!(rows
                .take(2)
                .all(|line| line.contains("Building 4 ") && line.contains('…')));
        }
        assert_eq!(panel.contains(ROUTER), width >= 200, "{width}:\n{panel}");
    }
}

/// A name with a tab and an escape in it can't move the cursor or break
/// the row: they are shown as spaces.
#[test]
fn control_characters_in_a_found_name_are_spaces() {
    let mut app = who_has_app();
    let search = app.who_has.as_mut().unwrap();
    search.found[0].name = "Fan\t1\u{1b}[2J".into();
    let frame = screen(&mut app, 120, 40);
    assert!(panel(&frame).contains("Fan 1 [2J"), "{frame}");
    assert!(!frame.contains(['\t', '\u{1b}']));
}
