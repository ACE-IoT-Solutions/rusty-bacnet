//! The MS/TP screen and the join confirmation (#969), from synthetic
//! reports and keys, with their frames; the sampler on a real handle.

use bacnet_transport::mstp::MstpDiagnosticsSnapshot;
use insta::assert_snapshot;

use super::*;
use crate::mstp::MstpArgs;
use crate::tui::app::connection::Connection;
use crate::tui::app::mstp::JoinDialog;
use crate::tui::app::watch::Screen;
use crate::tui::app::{Link, Overlay};
use crate::tui::mstp::{MstpReport, NodeView, Sample};
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile};

fn settings() -> MstpArgs {
    MstpArgs {
        port: "/dev/ttyUSB0".into(),
        baud: 38_400,
        mac: 3,
        max_master: 127,
        max_info_frames: 1,
    }
}

/// A flags-only MS/TP app, `join` its confirmation.
fn mstp_app(join: Option<JoinDialog>) -> App {
    App::new(
        AppConfig {
            transport: "MSTP",
            style: AddressStyle::Mstp,
            color: false,
            picker: None,
            join,
            passive: false,
            now: t0(),
        },
        LogRing::new(100),
    )
}

/// Counters as a fake diagnostics handle would report them.
fn counters(scale: u64) -> MstpDiagnosticsSnapshot {
    let mut c = MstpDiagnosticsSnapshot::default();
    c.mac.frames_received = 1200 * scale;
    c.mac.frames_transmitted = 300 * scale;
    c.mac.npdus_delivered = 40 * scale;
    c.mac.token_rotations = 250 * scale;
    c.mac.token_retries = scale;
    c.mac.parser_errors = 2 * scale;
    c.wait_for_reply_timeouts = scale;
    c.reply_postponed_tx = 3 * scale;
    c
}

fn sample(scale: u64, sole_master: bool) -> Box<Sample> {
    Box::new(Sample {
        node: Some(NodeView {
            state: "USE_TOKEN",
            holds_token: true,
            next_station: 7,
            poll_station: 8,
            sole_master,
        }),
        counters: counters(scale),
    })
}

/// An app on the trunk, with one reading, on the MS/TP screen.
fn joined() -> App {
    let mut app = mstp_app(None);
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "mstp:3".into(),
        },
    );
    worker(&mut app, WorkerEvent::Mstp(MstpReport::Joined(settings())));
    worker(
        &mut app,
        WorkerEvent::Mstp(MstpReport::Sample(sample(1, false))),
    );
    press(&mut app, KeyCode::Char('6'));
    assert_eq!(app.screen, Screen::Mstp);
    app
}

fn value(app: &App, label: &str) -> u64 {
    let rows = app.mstp.counters().expect("a reading");
    rows.into_iter()
        .find(|(name, _)| *name == label)
        .unwrap_or_else(|| panic!("no row {label}"))
        .1
}

#[test]
fn the_panel_shows_the_station_its_token_state_and_counters() {
    let mut app = joined();
    let text = screen(&mut app, 120, 40);
    for want in [
        "Port         /dev/ttyUSB0",
        "Station      mstp:3",
        "Baud         38400",
        "Token        HELD",
        "State        USE_TOKEN",
        "Next station mstp:7",
        "Sole master  no",
        "Tokens passed",
        "since joining, live",
    ] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    assert_snapshot!("mstp_panel_120x40", text);
    assert_snapshot!("mstp_panel_80x24", screen(&mut app, 80, 24));
}

#[test]
fn a_sole_master_is_called_out() {
    let mut app = joined();
    worker(
        &mut app,
        WorkerEvent::Mstp(MstpReport::Sample(sample(1, true))),
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("SOLE MASTER"), "{text}");
}

/// `z` counts from now; Space holds the screen and then catches up.
#[test]
fn clear_counts_from_now_and_pause_holds_the_screen() {
    let mut app = joined();
    assert_eq!(value(&app, "Tokens passed"), 250);
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(value(&app, "Tokens passed"), 0);
    worker(
        &mut app,
        WorkerEvent::Mstp(MstpReport::Sample(sample(3, false))),
    );
    assert_eq!(value(&app, "Tokens passed"), 500);
    assert_eq!(value(&app, "Frames received"), 2400);
    assert!(screen(&mut app, 120, 40).contains("since clear, live"));

    press(&mut app, KeyCode::Char(' '));
    worker(
        &mut app,
        WorkerEvent::Mstp(MstpReport::Sample(sample(5, false))),
    );
    assert_eq!(value(&app, "Tokens passed"), 500, "paused");
    assert!(screen(&mut app, 120, 40).contains("PAUSED"));
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(value(&app, "Tokens passed"), 1000, "caught up");
}

#[test]
fn without_mstp_the_screen_says_how_to_get_it() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('6'));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("MS/TP only: start bacnet tui with --mstp"),
        "{text}"
    );
}

/// Joining is T2: nothing but `y` joins, and it joins on the port chosen.
#[test]
fn joining_from_flags_needs_y_and_takes_the_port_chosen() {
    let listed = vec!["/dev/ttyUSB1".to_string(), "/dev/ttyUSB0".to_string()];
    let mut app = mstp_app(Some(JoinDialog::for_flags(settings(), listed)));
    assert_eq!(app.link, Link::Picking);
    assert_snapshot!("mstp_join_80x24", screen(&mut app, 80, 24));
    for key in [
        KeyCode::Enter,
        KeyCode::Char('j'),
        KeyCode::Char('c'),
        KeyCode::Char(' '),
    ] {
        assert!(press(&mut app, key).is_empty(), "{key:?} must not join");
    }
    assert!(matches!(app.overlay, Overlay::Join(_)));
    assert_eq!(
        press(&mut app, KeyCode::Char('y')),
        [Command::JoinTrunk {
            port: "/dev/ttyUSB1".into()
        }],
        "the port moved to: the configured one first, then the listed ones without repeats"
    );
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(app.link, Link::Connecting);

    // Declining a flags-only join leaves nothing to do: it quits, sending
    // nothing.
    let mut app = mstp_app(Some(JoinDialog::for_flags(settings(), vec![])));
    assert!(press(&mut app, KeyCode::Esc).is_empty());
    assert!(app.quit);
}

/// A profile's connection waits for the same confirmation; declining goes
/// back to the profile with nothing sent.
#[test]
fn an_mstp_profile_connects_only_after_the_join_is_confirmed() {
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    let mut profile = Profile::mstp_default();
    profile.mstp_mut().port = "/dev/ttyUSB0".into();
    profile.mstp_mut().mac = 3;
    profile.authorized_at_unix = Some(1);
    profile.authorized_safety = Some(profile.safety.clone());
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "trunk".into(),
            profile: profile.clone(),
            checks: Default::default(),
        })),
    );
    assert!(press(&mut app, KeyCode::Char('c')).is_empty(), "held");
    let Overlay::Join(dialog) = &app.overlay else {
        panic!("expected the join confirmation, got {:?}", app.overlay);
    };
    assert_eq!(dialog.ports, ["/dev/ttyUSB0"]);
    assert_eq!(app.link, Link::Down);

    assert!(press(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.overlay, Overlay::Connection);
    assert!(!app.connection.as_ref().unwrap().pending);

    press(&mut app, KeyCode::Char('c'));
    let sent = press(&mut app, KeyCode::Char('y'));
    let [Command::Profile(Request::Connect {
        name,
        profile: sent,
        ..
    })] = sent.as_slice()
    else {
        panic!("unexpected {sent:?}");
    };
    assert_eq!((name.as_str(), sent), ("trunk", &profile));
    assert_eq!(
        (app.transport, app.link.clone()),
        ("MSTP", Link::Connecting)
    );
}

/// The sampler reads the handle once a second and stops offering when the
/// session ends.
#[tokio::test(start_paused = true)]
async fn the_sampler_reports_the_handle_each_second() {
    use crate::tui::worker::EventSink;
    let (serial, _peer) = bacnet_transport::mstp::LoopbackSerial::pair();
    let transport = bacnet_transport::mstp::MstpTransport::new(serial, settings().config());
    let (sink, mut events) = EventSink::channel(8);
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let scoped = sink.for_session(1, cancelled);
    let task = tokio::spawn(crate::tui::mstp::sample(transport.diagnostics(), scoped));
    for _ in 0..2 {
        let event = events.recv().await.unwrap();
        let WorkerEvent::Session {
            generation: 1,
            event,
        } = event
        else {
            panic!("unexpected {event:?}");
        };
        assert_eq!(
            *event,
            WorkerEvent::Mstp(MstpReport::Sample(Box::new(Sample {
                node: None,
                counters: MstpDiagnosticsSnapshot::default(),
            })))
        );
    }
    cancel.send_replace(true);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(events.try_recv().is_err(), "nothing after the session ends");
    task.abort();
}

/// A flags-only MS/TP session opens nothing until the join is confirmed,
/// then opens the port the confirmation chose.
#[cfg(feature = "serial")]
#[tokio::test]
async fn the_coordinator_joins_only_on_confirmation_and_on_the_port_chosen() {
    use super::connections::event;
    use crate::tui::worker::session::{ConnectPlan, TransportKind};
    use crate::tui::worker::{coordinator, EventSink};
    let dir = tempfile::tempdir().unwrap();
    let missing = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let mut cli_settings = settings();
    cli_settings.port = missing("configured");
    let mut args = crate::tui::profile::Profile::default()
        .transport_args()
        .unwrap();
    args.mstp = Some(cli_settings);
    let plan = ConnectPlan {
        kind: TransportKind::Mstp,
        args,
        await_interface: true,
    };
    let (sink, mut events) = EventSink::channel(32);
    let (commands, rx) = tokio::sync::mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
    commands.send(Command::Reconnect).await.unwrap();
    let WorkerEvent::Rejected { error } = event(&mut events).await else {
        panic!("Reconnect must not join");
    };
    assert!(error.contains("confirm joining the trunk"), "{error}");
    let chosen = missing("chosen");
    commands
        .send(Command::JoinTrunk {
            port: chosen.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        event(&mut events).await,
        WorkerEvent::SessionStarted { generation: 1 }
    );
    loop {
        if let WorkerEvent::Session { event, .. } = event(&mut events).await {
            if let WorkerEvent::ConnectFailed { error } = *event {
                assert!(error.contains(&chosen), "{error}");
                break;
            }
        }
    }
    drop((commands, events));
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}
