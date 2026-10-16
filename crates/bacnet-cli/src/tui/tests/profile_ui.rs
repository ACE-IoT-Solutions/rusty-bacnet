//! Profile UI state transitions, requests and rendered minimum-size content.

use super::*;
use crate::tui::app::connection::{Connection, Screen};
use crate::tui::app::{Link, Overlay};
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile};
use insta::assert_snapshot;

fn profile_app() -> App {
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    app
}

#[test]
fn create_save_preview_and_attest_are_separate_explicit_actions() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "loopback");
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Profile(Request::Save {
        name,
        profile,
        create: true,
        previous: None,
    })] = commands.as_slice()
    else {
        panic!("unexpected {commands:?}");
    };
    assert_eq!(name, "loopback");
    assert_eq!(app.link, Link::Down, "save does not connect");
    let profile = profile.clone();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: name.clone(),
            profile: profile.clone(),
        })),
    );
    assert_eq!(app.connection.as_ref().unwrap().screen, Screen::Preview);
    assert!(
        press(&mut app, KeyCode::Enter).is_empty(),
        "ordinary Enter is not first-use attestation"
    );
    assert!(app
        .connection
        .as_ref()
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("attest"));
    assert_eq!(
        press(&mut app, KeyCode::Char('a')),
        [Command::Profile(Request::Connect {
            name: "loopback".into(),
            profile,
            attest: true
        })]
    );
    assert_eq!(app.link, Link::Connecting);
}

#[test]
fn invalid_or_failed_save_retains_editable_fields_and_never_connects() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    assert!(app.connection.as_ref().unwrap().error.is_some());
    type_text(&mut app, "site");
    press(&mut app, KeyCode::Enter);
    worker(
        &mut app,
        WorkerEvent::Profile(Err("disk is read-only".into())),
    );
    let c = app.connection.as_ref().unwrap();
    assert_eq!(c.screen, Screen::Edit);
    assert_eq!(c.fields[0], "site");
    assert!(!c.pending);
    assert_eq!(c.error.as_deref(), Some("disk is read-only"));
    assert_eq!(app.link, Link::Down);
}

#[test]
fn clearing_bbmd_returns_to_normal_and_network_edits_require_new_attestation() {
    let mut app = profile_app();
    let mut p = Profile::default();
    p.bip.bbmd = Some("127.0.0.2:47808".parse().unwrap());
    p.bip.renewal_secs = Some(12);
    p.authorized_at_unix = Some(1_794_000_000);
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "site".into(),
            profile: p.clone(),
        })),
    );
    press(&mut app, KeyCode::Char('e'));
    for _ in 0..5 {
        press(&mut app, KeyCode::Tab);
    }
    assert_eq!(app.connection.as_ref().unwrap().focus, 6);
    for _ in 0..30 {
        press(&mut app, KeyCode::Backspace);
    }
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Profile(Request::Save {
        profile,
        previous: Some(previous),
        ..
    })] = commands.as_slice()
    else {
        panic!("{commands:?}");
    };
    assert_eq!(previous, &p);
    assert!(profile.bip.bbmd.is_none() && profile.bip.renewal_secs.is_none());
    assert!(profile.authorized_at_unix.is_none());
}

#[test]
fn list_selection_and_reload_request_disk_state_without_connecting() {
    let mut app = profile_app();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Catalog {
            names: vec!["alpha".into(), "beta".into()],
            root: "/config".into(),
        })),
    );
    press(&mut app, KeyCode::Down);
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        [Command::Profile(Request::Load("beta".into()))]
    );
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "beta".into(),
            profile: Profile::default(),
        })),
    );
    assert_eq!(
        press(&mut app, KeyCode::Char('r')),
        [Command::Profile(Request::Load("beta".into()))]
    );
    assert_eq!(app.link, Link::Down);
}

#[test]
fn profile_frames_at_minimum_and_full_size() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = profile_app();
        worker(
            &mut app,
            WorkerEvent::Profile(Ok(Reply::Catalog {
                names: vec!["lab-east".into(), "lab-west".into()],
                root: "/config/bacnet".into(),
            })),
        );
        assert_snapshot!(format!("profile_picker_{w}x{h}"), screen(&mut app, w, h));
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "lab-local");
        assert_snapshot!(format!("profile_editor_{w}x{h}"), screen(&mut app, w, h));
        let mut p = Profile::default();
        p.bip.interface = "127.0.0.1".parse().unwrap();
        p.bip.port = 0;
        p.bip.broadcast = "127.0.0.255".parse().unwrap();
        p.bip.bbmd = Some("127.0.0.2:47808".parse().unwrap());
        worker(
            &mut app,
            WorkerEvent::Profile(Ok(Reply::Loaded {
                name: "lab-local".into(),
                profile: p,
            })),
        );
        let text = screen(&mut app, w, h);
        assert!(
            text.contains("I am authorized") && text.contains("a attest") && text.contains("BBMD:")
        );
        assert_snapshot!(format!("profile_authorization_{w}x{h}"), text);
    }
}

#[test]
fn q_quits_picker_and_preview_but_types_in_the_editor() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert_eq!(app.connection.as_ref().unwrap().fields[0], "q");
}
