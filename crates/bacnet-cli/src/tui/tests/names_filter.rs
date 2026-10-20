//! Filtering and sorting by name (#1929): the `/` filter on the device
//! table and the object list matches cached names, and Name joins the
//! device table's sort cycle with names not known yet last. Neither asks
//! for a name.

use ratatui::crossterm::event::KeyCode;

use super::names::{ai, asks, browsing, device, frame_commands, named, names, table};
use super::*;
use crate::tui::app::devices::SortKey;
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};

/// An I-Have from device `instance` at 10.0.0.`instance` naming its
/// Device object.
fn device_named(app: &mut App, instance: u32, name: &str) {
    let found = FoundRow {
        device: instance,
        object: device(instance),
        name: name.into(),
        address: format!("10.0.0.{instance}:47808"),
        via: None,
        network: None,
    };
    worker(app, WorkerEvent::Discovery(DiscoveryEvent::IHave(found)));
}

/// The filter finds a device by a name the cache already holds, even one
/// far off screen, and asks for no name; a name learned later joins it.
#[test]
fn the_device_filter_matches_cached_names_only() {
    let mut app = table(60);
    device_named(&mut app, 55, "Boiler Plant");
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "BOILER");
    assert_eq!(visible(&mut app), [55]);
    // The one row on screen is named; nothing else is asked for.
    let asked = asks(&frame_commands(&mut app, 80, 24));
    assert!(
        asked.iter().all(|(_, objects)| objects.is_empty()),
        "{asked:?}"
    );
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Boiler Plant"), "{text}");
    device_named(&mut app, 7, "Boiler 2");
    assert_eq!(visible(&mut app), [7, 55]);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(visible(&mut app).len(), 60);
}

/// Name joins the sort cycle after Last seen; names not known yet sort
/// last in either direction.
#[test]
fn the_device_table_sorts_by_name_with_pending_names_last() {
    let mut app = table(4);
    device_named(&mut app, 3, "alpha");
    device_named(&mut app, 1, "Charlie");
    names(&mut app, 4, vec![(device(4), named("bravo"))]);
    for _ in 0..5 {
        press(&mut app, KeyCode::Char('s'));
    }
    assert_eq!(app.devices.sort(), (SortKey::Name, false));
    assert_eq!(visible(&mut app), [3, 4, 1, 2]);
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(visible(&mut app), [1, 4, 3, 2], "pending still last");
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("sort: name"), "{text}");
    assert!(text.contains("Name*"), "{text}");
    // A name arriving moves its row.
    device_named(&mut app, 2, "delta");
    assert_eq!(visible(&mut app), [2, 1, 4, 3]);
}

/// The object list's filter matches cached names as well as labels.
#[test]
fn the_object_filter_matches_cached_names() {
    let mut app = browsing(5);
    names(
        &mut app,
        7,
        vec![(ai(3), named("Supply Fan")), (ai(1), named("Zone Temp"))],
    );
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "fan");
    let objects = &app.browser.as_ref().unwrap().objects;
    assert_eq!(objects.shown(), 1);
    assert_eq!(objects.selected(), Some(ai(3)));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Objects (1 of 5)"), "{text}");
    // A name learned while the filter is on joins the match.
    names(&mut app, 7, vec![(ai(4), named("Exhaust fan"))]);
    app.settle();
    assert_eq!(app.browser.as_ref().unwrap().objects.shown(), 2);
}
