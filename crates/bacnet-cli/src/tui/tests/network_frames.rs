//! The Network screen's frames at 120x40, from synthetic worker events at
//! fixed times, and the app's side of it: keys, the countdown, the
//! refusal banner and what a read asks the worker for.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};
use bacnet_transport::bvll::BvlcResultCode;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::network::View;
use crate::tui::app::watch::Screen;
use crate::tui::network::matrix::Probe;
use crate::tui::network::{BdtRow, FdtRow, Foreign, NetworkCommand, NetworkEvent};

fn at_ip(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
}

const LOCAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 9, 5), 47808);

fn foreign() -> Foreign {
    Foreign {
        bbmd: at_ip(1),
        ttl: 60,
        renewal: Duration::from_secs(30),
    }
}

fn snapshot(outcome: ForeignRegistrationOutcome) -> ForeignRegistrationSnapshot {
    let mut s = ForeignRegistrationSnapshot::default();
    s.last_bbmd = Some(at_ip(1));
    s.last_ttl = Some(60);
    s.last_outcome = Some(outcome);
    s.attempts = 2;
    if outcome == ForeignRegistrationOutcome::Refused {
        s.refused = 2;
        s.last_result = Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
        s.next_attempt_in = Some(Duration::from_secs(30));
    } else {
        s.accepted = 2;
        s.last_result = Some(BvlcResultCode::SUCCESSFUL_COMPLETION);
        s.registered = true;
        s.lease_remaining = Some(Duration::from_secs(48));
        s.next_attempt_in = Some(Duration::from_secs(18));
    }
    s
}

fn bdt_row(last: u8, mask: [u8; 4]) -> BdtRow {
    BdtRow {
        address: at_ip(last),
        mask: Ipv4Addr::from(mask),
    }
}

fn fdt_row(address: SocketAddrV4, remaining: u16) -> FdtRow {
    FdtRow {
        address,
        ttl: 60,
        remaining,
    }
}

/// Registered with 10.0.1.1 at 10 s, its tables read at 12 s, viewed at
/// 15 s.
fn registered() -> App {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    press(&mut app, KeyCode::Char('4'));
    tick(&mut app, at(10));
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Accepted));
    worker(&mut app, WorkerEvent::Network(event));
    tick(&mut app, at(12));
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(vec![
            bdt_row(1, [255; 4]),
            bdt_row(2, [255; 4]),
            bdt_row(3, [255, 255, 255, 0]),
        ]),
        fdt: Ok(vec![
            fdt_row(SocketAddrV4::new(Ipv4Addr::new(10, 0, 7, 9), 47808), 40),
            fdt_row(LOCAL, 88),
        ]),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    tick(&mut app, at(15));
    app
}

#[test]
fn foreign_device_panel_and_bdt_120x40() {
    let mut app = registered();
    assert_eq!(app.network.own, Some(1));
    // 18 s at 10 s, so 13 s at 15 s.
    assert_eq!(
        app.network.next_attempt(app.now),
        Some(Duration::from_secs(13))
    );
    assert_eq!(
        app.network.lease_left(app.now),
        Some(Duration::from_secs(43))
    );
    assert_snapshot!("network_fd_bdt_120x40", screen(&mut app, 120, 40));
}

#[test]
fn fdt_view_marks_our_entry_120x40() {
    let mut app = registered();
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.network.view, View::Fdt);
    let text = screen(&mut app, 120, 40);
    // 88 s left at 12 s, so 85 s at 15 s.
    assert!(
        text.contains("10.0.9.5:47808         60 s      85 s      this client"),
        "{text}"
    );
    assert_snapshot!("network_fdt_120x40", text);
}

#[test]
fn matrix_flags_the_missing_entry_120x40() {
    let mut app = registered();
    let commands = press(&mut app, KeyCode::Char('m'));
    assert_eq!(
        commands,
        [Command::Network(NetworkCommand::Survey { seed: at_ip(1) })]
    );
    let probes = vec![
        (
            at_ip(1),
            Probe::Table(vec![bdt_row(1, [255; 4]), bdt_row(2, [255; 4])]),
        ),
        (at_ip(2), Probe::Table(vec![bdt_row(2, [255; 4])])),
    ];
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("#2 10.0.2.1:47808         MISS  uni"),
        "{text}"
    );
    assert!(
        text.contains(
            "10.0.1.1:47808 lists 10.0.2.1:47808, but 10.0.2.1:47808 does not list 10.0.1.1:47808"
        ),
        "{text}"
    );
    assert_snapshot!("network_matrix_120x40", text);
}

#[test]
fn a_refusal_shows_a_banner_on_every_screen() {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Refused));
    worker(&mut app, WorkerEvent::Network(event));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("REFUSED BBMD 10.0.1.1:47808 refused the foreign-device registration (REGISTER_FOREIGN_DEVICE_NAK)"),
        "{text}"
    );
    press(&mut app, KeyCode::Char('4'));
    assert_snapshot!("network_refused_120x40", screen(&mut app, 120, 40));
    // Accepted again: the banner goes.
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Accepted));
    worker(&mut app, WorkerEvent::Network(event));
    assert_eq!(app.network.refusal(), None);
}

#[test]
fn keys_read_the_target_and_need_a_bbmd() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.screen, Screen::Network);
    assert!(press(&mut app, KeyCode::Char('r')).is_empty());
    assert!(app.flash.as_ref().unwrap().text.contains(":bbmd ADDRESS"));
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    let commands = press(&mut app, KeyCode::Enter);
    assert_eq!(
        commands,
        [Command::Network(NetworkCommand::Tables { bbmd: at_ip(4) })]
    );
    assert!(app.network.reading);
    // An answer for another BBMD is not shown as this one's.
    let stale = NetworkEvent::Tables {
        bbmd: at_ip(5),
        local: LOCAL,
        bdt: Ok(Vec::new()),
        fdt: Err("unreachable: no answer".into()),
    };
    worker(&mut app, WorkerEvent::Network(stale));
    assert_eq!(app.network.bdt, None);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.network.view, View::Matrix);
    assert_eq!(
        press(&mut app, KeyCode::Char('r')),
        [Command::Network(NetworkCommand::Survey { seed: at_ip(4) })]
    );
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
}

#[test]
fn other_transports_and_passive_sessions_read_nothing() {
    let mut app = App::new(
        AppConfig {
            transport: "BIP6",
            style: AddressStyle::Bip6,
            color: false,
            picker: None,
            passive: false,
            now: t0(),
        },
        LogRing::new(10),
    );
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "[::1]:47808".into(),
        },
    );
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    assert!(app.flash.as_ref().unwrap().text.contains("BACnet/IP"));
    assert!(screen(&mut app, 120, 40).contains("B/IP only"));

    let mut passive = App::new(
        AppConfig {
            transport: "BIP",
            style: AddressStyle::Bip,
            color: false,
            picker: None,
            passive: true,
            now: t0(),
        },
        LogRing::new(10),
    );
    passive.network.target = Some(at_ip(1));
    press(&mut passive, KeyCode::Char('4'));
    assert!(press(&mut passive, KeyCode::Char('m')).is_empty());
}

#[test]
fn answers_for_an_earlier_target_are_dropped() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    assert!(app.network.surveying);
    // `:bbmd` moves to another BBMD while the survey from 10.0.1.1 runs.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    press(&mut app, KeyCode::Enter);
    assert!(app.network.reading);
    let late = NetworkEvent::Survey {
        seed: at_ip(1),
        probes: vec![(at_ip(1), Probe::Table(vec![bdt_row(1, [255; 4])]))],
    };
    worker(&mut app, WorkerEvent::Network(late));
    assert_eq!(app.network.matrix, None, "the old survey's matrix shows");
    // The automatic read of the registration's BBMD, or a refusal of an
    // earlier request, leaves the new read waiting.
    let auto = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(Vec::new()),
        fdt: Ok(Vec::new()),
    };
    worker(&mut app, WorkerEvent::Network(auto));
    let refused = NetworkEvent::Refused {
        command: NetworkCommand::Survey { seed: at_ip(1) },
        reason: "busy".into(),
    };
    worker(&mut app, WorkerEvent::Network(refused));
    assert!(app.network.reading);
    let refused = NetworkEvent::Refused {
        command: NetworkCommand::Tables { bbmd: at_ip(4) },
        reason: "busy".into(),
    };
    worker(&mut app, WorkerEvent::Network(refused));
    assert!(!app.network.reading);
}

#[test]
fn a_large_matrix_shows_what_fits_and_counts_the_rest() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let everyone: Vec<BdtRow> = (1..=20).map(|last| bdt_row(last, [255; 4])).collect();
    let probes = (1..=20)
        .map(|last| (at_ip(last), Probe::Table(everyone.clone())))
        .collect();
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("#14   +6 more"), "{text}");
    assert!(!text.contains("#15"), "{text}");
    assert!(text.contains("#14 10.0.14.1:47808"), "{text}");
    assert!(text.contains("All 20 BDTs agree"), "{text}");
}
