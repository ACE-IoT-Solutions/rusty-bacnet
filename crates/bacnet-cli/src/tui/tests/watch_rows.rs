//! Watch rows as an operator reads them (#1891): object names from the
//! first read, routed devices as `NET:INSTANCE`, whole object labels at
//! 80 columns and a trend that fills a wide terminal. The frames come from
//! synthetic worker reports; the name reads run against the lab device.

use std::time::Duration;

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::watch::{Screen, HISTORY};
use crate::tui::watched::{RowId, Sample, Source, WatchEvent};

/// Type `line` on the command bar and run it.
fn command(app: &mut App, line: &str) -> Vec<Command> {
    press(app, KeyCode::Char(':'));
    type_text(app, line);
    press(app, KeyCode::Enter)
}

fn watch(app: &mut App, event: WatchEvent) {
    worker(app, WorkerEvent::Watch(event));
}

fn sample(app: &App, value: &str, number: Option<f64>) -> Sample {
    Sample {
        value: Ok(value.into()),
        number,
        flags: None,
        at: app.now,
    }
}

/// Four rows: a routed analog input with a long name and a long history,
/// a named binary output showing its state text, a named multi-state
/// output, and an analog value whose device gave no name.
fn four_rows() -> App {
    let mut app = connected_app();
    let mut routed = row(2001, "10.0.0.9:47808", 1, 0);
    routed.network = Some(2);
    worker(&mut app, WorkerEvent::Discovered(routed));
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 1, 0)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1002, "10.0.0.3:47808", 1, 0)),
    );
    for line in [
        "watch 2:2001 ai:1",
        "watch 1001 bo:1",
        "watch 1001 mso:1",
        "watch 1002 av:7",
    ] {
        command(&mut app, line);
    }
    assert_eq!(app.watch.rows().len(), 4);
    let details: [(RowId, Option<&str>, Option<&str>); 4] = [
        (
            1,
            Some("Discharge Air Temperature North Wing"),
            Some("degrees-fahrenheit"),
        ),
        (2, Some("Supply Fan"), None),
        (3, Some("Fan Speed"), None),
        (4, None, Some("percent")),
    ];
    for (row, name, units) in details {
        watch(
            &mut app,
            WatchEvent::Details {
                row,
                name: name.map(Into::into),
                units: units.map(Into::into),
            },
        );
    }
    watch(
        &mut app,
        WatchEvent::Source {
            row: 1,
            source: Source::Polled("routed device".into()),
        },
    );
    for row in 2..=4 {
        watch(
            &mut app,
            WatchEvent::Source {
                row,
                source: Source::Cov { lifetime: 300 },
            },
        );
    }
    let history: Vec<(RowId, Sample)> = (0..HISTORY)
        .map(|n| {
            let value = 70.0 + (n % 16) as f64 * 0.5;
            (1, sample(&app, &format!("{value} °F"), Some(value)))
        })
        .collect();
    watch(&mut app, WatchEvent::Samples(history));
    let rest = vec![
        (2, sample(&app, "active (Running)", Some(1.0))),
        (3, sample(&app, "2 (Medium)", Some(2.0))),
        (4, sample(&app, "41.25 %", Some(41.25))),
    ];
    watch(&mut app, WatchEvent::Samples(rest));
    app.screen = Screen::Watch;
    app
}

/// At 80, 120 and 200 columns every row keeps its whole device and object
/// label; the name takes the room left; at 200 the trend fills its column.
#[test]
fn watch_rows_keep_what_identifies_them_at_every_width() {
    let mut app = four_rows();
    for (w, h) in [(80, 24), (120, 40), (200, 50)] {
        let text = screen(&mut app, w, h);
        let lines: Vec<&str> = text.lines().collect();
        for (at, (device, object)) in [
            ("2:2001", "analog-input 1"),
            ("1001", "binary-output 1"),
            ("1001", "multi-state-output 1"),
            ("1002", "analog-value 7"),
        ]
        .into_iter()
        .enumerate()
        {
            let line = lines[3 + at];
            assert!(
                line.contains(&format!("│{device} ")) && line.contains(&format!("{object} ")),
                "{w}x{h}: {line}"
            );
        }
        assert!(text.contains("active (Running)"), "{text}");
        // The routed row's trend runs to the border.
        let routed = lines[3].trim_end_matches(['"', '│']);
        assert!(
            routed.ends_with(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']),
            "{w}x{h}: {routed}"
        );
        assert_snapshot!(format!("watch_rows_{w}x{h}"), text);
    }
    // The history fills the trend of a terminal 600 columns wide too.
    let text = screen(&mut app, 600, 24);
    let routed = text.lines().nth(3).unwrap().trim_end_matches(['"', '│']);
    assert!(
        routed.ends_with(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']),
        "{routed}"
    );
}

/// `2001` and `2:2001` are one row: the device table gives the network.
#[test]
fn a_routed_device_is_one_row_however_it_is_typed() {
    let mut app = four_rows();
    command(&mut app, "watch 2001 ai:1");
    assert_eq!(app.watch.rows().len(), 4);
    let mut app = connected_app();
    let mut routed = row(2001, "10.0.0.9:47808", 1, 0);
    routed.network = Some(2);
    worker(&mut app, WorkerEvent::Discovered(routed));
    command(&mut app, "watch 2001 ai:1");
    assert_eq!(app.watch.rows()[0].point.device_label(), "2:2001");
}

/// The lab device with `quirks`, discovered.
async fn lab(quirks: Quirks) -> Site {
    let server = Server {
        quirks,
        ..Server::lab(100, 100)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    site.who_is("", 1).await;
    site
}

/// Watch AI:1 of the lab device and run until its value is in.
async fn watch_ai1(site: &mut Site) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "watch 100 ai:1");
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| {
        app.watch
            .rows()
            .iter()
            .any(|row| !matches!(row.source, Source::Starting) && row.value.is_some())
    })
    .await;
}

fn reads(site: &Site) -> Vec<u8> {
    site.hub
        .services_to(&[10, 0, 0, 100, 0xBA, 0xC0])
        .into_iter()
        .filter(|service| [12, 14].contains(service))
        .collect()
}

/// The name comes with the first read: one ReadPropertyMultiple.
#[tokio::test(start_paused = true)]
async fn the_name_comes_with_the_first_read() {
    let mut site = lab(Quirks::default()).await;
    watch_ai1(&mut site).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.name.as_deref(), Some("Zone Temp"));
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    assert_eq!(reads(&site), [14]);
    site.stop().await;
}

/// A device that fails the whole first read for Object_Name is asked
/// again without it, and the row starts nameless.
#[tokio::test(start_paused = true)]
async fn a_device_that_fails_the_read_for_the_name_still_gets_its_row() {
    let mut site = lab(Quirks {
        refuse_object_name: true,
        ..Quirks::default()
    })
    .await;
    watch_ai1(&mut site).await;
    site.run_for(Duration::from_millis(500)).await;
    let row = &site.app.watch.rows()[0];
    assert!(matches!(row.source, Source::Cov { .. }), "{:?}", row.source);
    assert_eq!(row.name, None);
    assert!(row.value.clone().unwrap().is_ok(), "{:?}", row.value);
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    assert_eq!(reads(&site), [14, 14]);
    site.stop().await;
}

/// Read one property at a time, a refused Object_Name is that property's
/// error only: the value, flags and units still arrive.
#[tokio::test(start_paused = true)]
async fn a_refused_name_read_alone_leaves_the_rest_of_the_row() {
    let mut site = lab(Quirks {
        refuse_object_name: true,
        refuse_rpm: true,
        ..Quirks::default()
    })
    .await;
    watch_ai1(&mut site).await;
    site.run_for(Duration::from_millis(500)).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.name, None);
    assert!(row.value.clone().unwrap().is_ok(), "{:?}", row.value);
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    // The refused RPM, then four ReadProperty: value, flags, units, name.
    assert_eq!(reads(&site), [14, 12, 12, 12, 12]);
    site.stop().await;
}
