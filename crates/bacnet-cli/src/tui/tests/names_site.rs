//! Name reads (#1896) against the lab device and bare devices on the
//! in-memory site: through the per-device gate and the rate cap, the
//! ReadProperty fallback, refusals, timeouts, and rows withdrawn before
//! their turn.

use std::time::Duration;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use ratatui::crossterm::event::KeyCode;

use super::names::{ai, asks, frame_commands};
use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::browser::ViewState;
use crate::tui::app::names::Name;
use crate::tui::point::{BrowseResult, NameAsk, ReqId};
use crate::tui::worker::browse::names::NAMES_PER_RPM;
use crate::tui::worker::session::SessionOptions;

const LAB: u32 = 100;
const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];
const RP: u8 = 12;
const RPM: u8 = 14;

fn capped(per_second: u32) -> SessionOptions {
    SessionOptions {
        max_rate: per_second,
        ..SessionOptions::default()
    }
}

/// Ask for the names on screen, as the UI loop does, and apply results
/// until every one is known.
async fn read_names(site: &mut Site) -> Vec<ObjectIdentifier> {
    let commands = frame_commands(&mut site.app, 200, 50);
    let asked: Vec<_> = asks(&commands).into_iter().flat_map(|(_, o)| o).collect();
    site.send(commands).await;
    known(site, asked.clone()).await;
    asked
}

/// Apply results until every one of `objects` on the lab device is known.
async fn known(site: &mut Site, objects: Vec<ObjectIdentifier>) {
    site.pump_until(move |app| objects.iter().all(|o| app.names.get(LAB, *o).is_some()))
        .await;
}

/// The services of the requests to the lab device since the `from`th.
fn services_since(site: &Site, from: usize) -> Vec<u8> {
    site.hub.requests_to(&LAB_MAC)[from..]
        .iter()
        .map(|(service, _)| *service)
        .collect()
}

/// The command a frame showing `objects` of the lab device would send.
fn ask(req: ReqId, objects: &[ObjectIdentifier]) -> Command {
    let asks = vec![NameAsk {
        device: LAB,
        objects: objects.to_vec(),
        retry: false,
    }];
    Command::Names { req, asks }
}

fn lab(quirks: Quirks) -> Server {
    Server {
        quirks,
        ..Server::lab(100, LAB)
    }
}

/// The lab device's names come by ReadPropertyMultiple, a few objects a
/// request, one request outstanding at a time and each after its slot in
/// the rate cap. The device's own name came with its card.
#[tokio::test(start_paused = true)]
async fn names_are_read_through_the_gate_and_the_rate_cap() {
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), capped(4)).await;
    super::browser::open_device(&mut site).await;
    let before = site.hub.requests_to(&LAB_MAC).len();
    let asked = read_names(&mut site).await;
    assert_eq!(asked.len(), 10, "every object but the device: {asked:?}");
    let ao1 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    assert_eq!(
        site.app.names.get(LAB, ao1),
        Some(&Name::Known("Damper".into()))
    );
    let rpms = asked.len().div_ceil(NAMES_PER_RPM);
    assert_eq!(services_since(&site, before), vec![RPM; rpms]);
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    let requests = site.hub.requests_to(&LAB_MAC);
    for pair in requests.windows(2) {
        let gap = pair[1].1 - pair[0].1;
        assert!(gap >= Duration::from_millis(250), "{gap:?}");
    }
    // Nothing is read twice.
    assert!(read_names(&mut site).await.is_empty());
    assert_eq!(site.hub.requests_to(&LAB_MAC).len(), requests.len());
    site.stop().await;
}

/// Which browser results arrive, in order, until `done` says so.
async fn results_until(
    site: &mut Site,
    mut done: impl FnMut(&[&'static str]) -> bool,
) -> Vec<&'static str> {
    let mut seen = Vec::new();
    site.pump_events_until(|event| {
        let event = match event {
            WorkerEvent::Session { event, .. } => event,
            other => other,
        };
        match event {
            WorkerEvent::Browse {
                result: BrowseResult::Names { names, .. },
                ..
            } => seen.extend(names.iter().map(|_| "name")),
            WorkerEvent::Browse {
                result: BrowseResult::Object(_),
                ..
            } => seen.push("object"),
            _ => {}
        }
        done(&seen)
    })
    .await;
    seen
}

fn names_after_object(seen: &[&str]) -> usize {
    let at = seen
        .iter()
        .position(|s| *s == "object")
        .unwrap_or(seen.len());
    seen[at..].iter().filter(|s| **s == "name").count()
}

/// Without a rate cap, name reads still share the device's gate with an
/// object read asked just before them: one request outstanding at a
/// time, the object read answered first, and the names after it.
#[tokio::test(start_paused = true)]
async fn names_and_an_object_read_take_turns_at_the_device() {
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), capped(0)).await;
    super::browser::open_device(&mut site).await;
    let mut commands = press(&mut site.app, KeyCode::Enter);
    let names = frame_commands(&mut site.app, 200, 50);
    let asked: Vec<_> = asks(&names).into_iter().flat_map(|(_, o)| o).collect();
    assert!(!asked.is_empty());
    commands.extend(names);
    site.send(commands).await;
    let count = asked.len();
    let seen = results_until(&mut site, |seen| names_after_object(seen) == count).await;
    assert_eq!(seen[0], "object", "{seen:?}");
    assert!(matches!(
        site.app.browser.as_ref().unwrap().view,
        ViewState::Ready(_)
    ));
    assert!(asked.iter().all(|o| site.app.names.get(LAB, *o).is_some()));
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}

/// An object read asked while names are being read stops the name job
/// between requests; it resumes after the object read and finishes.
#[tokio::test(start_paused = true)]
async fn an_object_read_stops_the_names_and_they_resume() {
    let servers = vec![lab(Quirks {
        refuse_rpm: true,
        ..Quirks::default()
    })];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), capped(4)).await;
    super::browser::open_device(&mut site).await;
    let names = frame_commands(&mut site.app, 200, 50);
    let asked: Vec<_> = asks(&names).into_iter().flat_map(|(_, o)| o).collect();
    site.send(names).await;
    site.run_for(Duration::from_millis(1_100)).await;
    let read = asked
        .iter()
        .filter(|o| site.app.names.get(LAB, **o).is_some())
        .count();
    assert!(read > 0 && read < asked.len(), "{read} of {}", asked.len());
    site.press(KeyCode::Enter).await;
    let left = asked.len() - read;
    let seen = results_until(&mut site, |seen| names_after_object(seen) == left).await;
    assert!(
        seen[0] == "object" || seen[..2] == ["name", "object"],
        "{seen:?}"
    );
    assert!(asked.iter().all(|o| site.app.names.get(LAB, *o).is_some()));
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}

/// A device that refuses RPM gives its names one ReadProperty each.
#[tokio::test(start_paused = true)]
async fn names_fall_back_to_read_property() {
    let server = lab(Quirks {
        refuse_rpm: true,
        ..Quirks::default()
    });
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    super::browser::open_device(&mut site).await;
    let before = site.hub.requests_to(&LAB_MAC).len();
    let asked = read_names(&mut site).await;
    assert_eq!(services_since(&site, before), vec![RP; asked.len()]);
    for object in &asked {
        let name = site.app.names.get(LAB, *object);
        assert!(matches!(name, Some(Name::Known(_))), "{object}: {name:?}");
    }
    site.stop().await;
}

/// A device that won't give names: one RPM fails as a whole, each object
/// then says so alone, and the names are kept as unavailable; the worker
/// doesn't read them again even when asked.
#[tokio::test(start_paused = true)]
async fn names_a_device_wont_give_are_read_once() {
    let server = lab(Quirks {
        refuse_object_name: true,
        ..Quirks::default()
    });
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    site.who_is("", 1).await;
    let objects = [ai(1), ai(2)];
    site.send(vec![ask(1_000, &objects)]).await;
    known(&mut site, objects.to_vec()).await;
    assert_eq!(services_since(&site, 0), [RPM, RP, RP]);
    for object in &objects {
        assert_eq!(site.app.names.get(LAB, *object), Some(&Name::Unavailable));
    }
    site.send(vec![ask(1_001, &objects)]).await;
    site.run_for(Duration::from_secs(5)).await;
    assert_eq!(services_since(&site, 0).len(), 3, "not read again");
    site.stop().await;
}

/// A device that stops answering times out once, and every name still
/// asked for is given up on as unreachable (not refused) rather than read
/// into more timeouts.
#[tokio::test(start_paused = true)]
async fn a_silent_device_times_out_once_for_all_its_names() {
    let server = lab(Quirks {
        silent: true,
        ..Quirks::default()
    });
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    site.who_is("", 1).await;
    let objects: Vec<_> = (1..=20).map(ai).collect();
    site.send(vec![ask(1_000, &objects)]).await;
    known(&mut site, objects.clone()).await;
    let services = services_since(&site, 0);
    assert!(
        !services.is_empty() && services.len() <= 4 && services.iter().all(|s| *s == RPM),
        "one RPM and its retries: {services:?}"
    );
    for object in &objects {
        assert_eq!(site.app.names.get(LAB, *object), Some(&Name::Unreachable));
    }
    site.stop().await;
}

/// A device that was away: its name times out and is left blank, and its
/// next I-Am has the name asked for, and read, again.
#[tokio::test(start_paused = true)]
async fn a_name_that_timed_out_is_read_after_the_next_i_am() {
    // One ReadProperty and its three retries go unanswered.
    let server = lab(Quirks {
        swallow_first: 4,
        ..Quirks::default()
    });
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    site.who_is("", 1).await;
    let me = super::names::device(LAB);
    let commands = frame_commands(&mut site.app, 120, 40);
    assert_eq!(asks(&commands), [(LAB, vec![me])]);
    site.send(commands).await;
    site.pump_until(|app| app.names.get(LAB, me).is_some())
        .await;
    assert_eq!(site.app.names.get(LAB, me), Some(&Name::Unreachable));
    assert!(frame_commands(&mut site.app, 120, 40).is_empty());
    site.who_is("", 1).await;
    let commands = frame_commands(&mut site.app, 120, 40);
    assert_eq!(asks(&commands), [(LAB, vec![me])]);
    site.send(commands).await;
    site.pump_until(|app| matches!(app.names.get(LAB, me), Some(Name::Known(_))))
        .await;
    assert_eq!(
        site.app.names.get(LAB, me),
        Some(&Name::Known("Lab device 100".into()))
    );
    site.stop().await;
}

/// The MAC of the bare device at 10.0.0.`octet`.
fn mac(octet: u8) -> [u8; 6] {
    [10, 0, 0, octet, 0xBA, 0xC0]
}

/// Scrolling quickly down a long device table at a low cap reads only the
/// rows still on screen: a row withdrawn while its job waited for a slot
/// costs neither a request nor a slot.
#[tokio::test(start_paused = true)]
async fn rows_scrolled_past_are_not_read() {
    let devices: Vec<(u8, u32)> = (10..50)
        .map(|octet| (octet, 1_000 + u32::from(octet)))
        .collect();
    let servers = devices
        .iter()
        .map(|&(octet, instance)| Server::bare(octet, instance))
        .collect();
    let mut site = Site::start_with_options(servers, ClientLimits::default(), capped(2)).await;
    site.who_is("", devices.len()).await;
    let commands = frame_commands(&mut site.app, 80, 24);
    let first = asks(&commands);
    site.send(commands).await;
    site.run_for(Duration::from_millis(10)).await;
    press(&mut site.app, KeyCode::End);
    let last = frame_commands(&mut site.app, 80, 24);
    let shown: Vec<u32> = asks(&last)
        .into_iter()
        .filter(|(_, objects)| !objects.is_empty())
        .map(|(device, _)| device)
        .collect();
    assert!(shown.len() > 10, "{shown:?}");
    site.send(last).await;
    // The rows on screen take one slot each at 2 a second, from the
    // first free one: none went to the rows scrolled past.
    let slots = Duration::from_millis(500) * u32::try_from(shown.len()).unwrap();
    site.run_for(slots + Duration::from_millis(500)).await;
    let mut read_off_screen = 0;
    for &(octet, instance) in &devices {
        let requests = site.hub.requests_to(&mac(octet)).len();
        if shown.contains(&instance) {
            let name = site.app.names.get(instance, super::names::device(instance));
            assert!(matches!(name, Some(Name::Known(_))), "{instance}: {name:?}");
        } else {
            read_off_screen += requests;
        }
    }
    assert!(
        read_off_screen <= 1,
        "{read_off_screen} requests to rows scrolled past"
    );
    assert!(first.len() > 10);
    site.stop().await;
}
