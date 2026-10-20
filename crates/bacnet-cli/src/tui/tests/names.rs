//! Object and device names (#1896): only the rows on screen are asked
//! for, once, and names that came free cost nothing. The asks come from
//! synthetic frames and worker events; the reads run against the lab
//! device through the browser's per-device gate and the rate cap.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::names::Name;
use crate::tui::app::objects::ListRow;
use crate::tui::app::watch::Screen;
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};
use crate::tui::point::{BrowseRead, BrowseResult, DeviceCard, ReqId};
use crate::tui::watched::WatchEvent;
use crate::tui::worker::browse::names::NAMES_PER_RPM;
use crate::tui::worker::session::SessionOptions;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// The name asks among `commands`: each device and what it is asked for.
fn asks(commands: &[Command]) -> Vec<(u32, Vec<ObjectIdentifier>)> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::Browse {
                device,
                read: BrowseRead::Names(objects),
                ..
            } => Some((*device, objects.clone())),
            _ => None,
        })
        .collect()
}

/// Draw a frame and tick: the commands for what it showed.
fn frame_commands(app: &mut App, width: u16, height: u16) -> Vec<Command> {
    render(app, width, height);
    let now = app.now;
    update(
        app,
        Action::Tick {
            now,
            dropped: app.dropped,
            log_generation: app.log.generation(),
        },
    )
}

/// Draw a frame and tick: the asks for what it showed.
fn frame_asks(app: &mut App, width: u16, height: u16) -> Vec<(u32, Vec<ObjectIdentifier>)> {
    asks(&frame_commands(app, width, height))
}

/// The request id of the one command among `commands`.
fn req_of(commands: &[Command]) -> ReqId {
    match commands {
        [Command::Browse { req, .. }] => *req,
        other => panic!("one ask: {other:?}"),
    }
}

/// The objects the browser's last frame showed.
fn on_screen(app: &App) -> Vec<ObjectIdentifier> {
    let objects = &app.browser.as_ref().unwrap().objects;
    objects
        .rows()
        .iter()
        .skip(objects.offset.get())
        .take(objects.page.get())
        .filter_map(|row| match row {
            ListRow::Object(object) => Some(*object),
            ListRow::Group { .. } => None,
        })
        .collect()
}

fn names(app: &mut App, device: u32, names: Vec<(ObjectIdentifier, Option<String>)>) {
    let result = BrowseResult::Names { device, names };
    worker(app, WorkerEvent::Browse { req: 0, result });
}

/// A connected app browsing device 7, whose Object_List holds `count`
/// analog inputs and whose card named it `Plant`.
fn browsing(count: u32) -> App {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(7, "10.0.0.7:47808", 260, 0)),
    );
    let [Command::Browse { req, .. }] = press(&mut app, KeyCode::Enter)[..] else {
        panic!("expected the device read");
    };
    let card = DeviceCard {
        name: Some("Plant".into()),
        ..DeviceCard::default()
    };
    let result = BrowseResult::Card(card);
    worker(&mut app, WorkerEvent::Browse { req, result });
    let result = BrowseResult::Objects {
        objects: (0..count).map(ai).collect(),
        indexed: false,
        stopped: None,
    };
    worker(&mut app, WorkerEvent::Browse { req, result });
    app
}

/// Of a 1,000-object list only the rows on screen are asked for, and a
/// frame showing the same rows asks nothing more.
#[test]
fn only_the_rows_on_screen_are_asked_for() {
    let mut app = browsing(1_000);
    let asked = frame_asks(&mut app, 80, 24);
    let shown = on_screen(&app);
    assert!(!shown.is_empty() && shown.len() < 30, "{shown:?}");
    assert_eq!(asked, [(7, shown.clone())]);
    assert!(frame_asks(&mut app, 80, 24).is_empty(), "asked already");
    // Names arriving for some of them change nothing either.
    let half: Vec<_> = shown[..shown.len() / 2]
        .iter()
        .map(|object| (*object, Some(format!("Point {}", object.instance_number()))))
        .collect();
    names(&mut app, 7, half);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("analog-input 0    Point 0"), "{text}");
    assert!(text.contains("…"), "a name still being read: {text}");
}

/// Scrolling replaces the ask with the rows now on screen, so rows that
/// scrolled off before their turn are never read; leaving the browser
/// stops the rest.
#[test]
fn scrolling_replaces_the_ask_and_closing_stops_it() {
    let mut app = browsing(1_000);
    let first = frame_asks(&mut app, 80, 24);
    let before = on_screen(&app);
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::PageDown);
    let after = frame_asks(&mut app, 80, 24);
    let shown = on_screen(&app);
    assert_ne!(shown, before);
    assert_eq!(after, [(7, shown.clone())], "the new rows only");
    assert!(first[0].1.iter().all(|object| !shown.contains(object)));
    // The device's own name came with its card: the device table asks
    // nothing for it, and leaving the browser withdraws the ask.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, Vec::new())]);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
}

/// Names that came free (the card, a watch row's first read, an I-Have)
/// are never asked for.
#[test]
fn names_that_came_free_are_not_asked_for() {
    let mut app = browsing(4);
    worker(
        &mut app,
        WorkerEvent::Discovered(row(8, "10.0.0.8:47808", 260, 0)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(9, "10.0.0.9:47808", 260, 0)),
    );
    // A watch row's first read names AI:1; an I-Have names AI:2 and,
    // unasked, device 8.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "watch 7 ai:1");
    press(&mut app, KeyCode::Enter);
    let row = app.watch.rows()[0].id;
    let details = WatchEvent::Details {
        row,
        name: Some("Supply Temp".into()),
        units: None,
    };
    worker(&mut app, WorkerEvent::Watch(details));
    for (on, object, name) in [(7, ai(2), "Return Temp"), (8, device(8), "AHU-8")] {
        let found = FoundRow {
            device: on,
            object,
            name: name.into(),
            address: format!("10.0.0.{on}:47808"),
            via: None,
            network: None,
        };
        worker(
            &mut app,
            WorkerEvent::Discovery(DiscoveryEvent::IHave(found)),
        );
    }
    app.screen = Screen::Browse;
    assert_eq!(frame_asks(&mut app, 120, 40), [(7, vec![ai(0), ai(3)])]);
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("Supply Temp") && text.contains("Return Temp"),
        "{text}"
    );
    // On the device table only device 9 is unnamed; 7 came with its card.
    app.screen = Screen::Main;
    assert_eq!(
        frame_asks(&mut app, 120, 40),
        [(7, Vec::new()), (9, vec![device(9)])]
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("Plant") && text.contains("AHU-8"), "{text}");
}

/// A name the device couldn't give is shown as such and not asked for
/// again, even after scrolling away and back, until `r` reads the device
/// afresh.
#[test]
fn a_name_the_device_cant_give_is_not_asked_again() {
    let mut app = browsing(3);
    assert_eq!(
        frame_asks(&mut app, 80, 24),
        [(7, vec![ai(0), ai(1), ai(2)])]
    );
    names(
        &mut app,
        7,
        vec![(ai(0), None), (ai(1), Some("Fan".into()))],
    );
    assert_eq!(app.names.get(7, ai(0)), Some(&Name::Unavailable));
    // A later failure doesn't lose a name.
    names(&mut app, 7, vec![(ai(1), None)]);
    assert_eq!(app.names.get(7, ai(1)), Some(&Name::Known("Fan".into())));
    assert!(frame_asks(&mut app, 80, 24).is_empty(), "AI:2 is still out");
    names(&mut app, 7, vec![(ai(2), None)]);
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Home);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("analog-input 0  -"), "{text}");
    // `r` on the object list reads the device again, names included.
    let refresh = press(&mut app, KeyCode::Char('r'));
    assert!(matches!(
        refresh.as_slice(),
        [Command::Browse {
            read: BrowseRead::Device,
            ..
        }]
    ));
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, vec![ai(0), ai(2)])]);
}

/// An ask the worker never took (a full queue, no session) is asked again
/// on the next frame, without a notice.
#[test]
fn an_ask_that_was_not_taken_is_asked_again() {
    let mut app = browsing(2);
    let commands = frame_commands(&mut app, 80, 24);
    req_of(&commands);
    let link = app.link.clone();
    app.command_rejected(commands[0].clone(), link, "full".into());
    assert!(app.flash.is_none(), "no notice for a background read");
    let again = frame_commands(&mut app, 80, 24);
    assert_eq!(asks(&again), [(7, vec![ai(0), ai(1)])]);
    // The coordinator's refusal comes back as a failed result.
    let req = req_of(&again);
    let result = BrowseResult::Failed("no active connection".into());
    worker(&mut app, WorkerEvent::Browse { req, result });
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, vec![ai(0), ai(1)])]);
}

/// The device table: a Name column at 80 columns that a long name fits
/// with `…`, all of it from the wide width; and none, with nothing asked,
/// when the other columns leave too little room.
#[test]
fn the_device_table_fits_its_name_column() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 260, 0)),
    );
    let long = "Air handler 1 supply fan and economizer, level 3 mechanical room";
    names(&mut app, 1001, vec![(device(1001), Some(long.into()))]);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Air handler 1 supply"), "{text}");
    assert!(!text.contains(long) && text.contains('…'), "{text}");
    let text = screen(&mut app, 200, 50);
    assert!(text.contains(long), "{text}");
    // A routed device's long address and router squeeze the name out.
    let mut squeezed = row(4_194_302, "65534:0x7F000001C335BAC0DEADBEEF", 65_535, 0);
    squeezed.network = Some(65_534);
    squeezed.via = Some("192.168.100.254:47808".into());
    worker(&mut app, WorkerEvent::Discovered(squeezed));
    let asked = frame_asks(&mut app, 80, 24);
    assert!(asked.is_empty(), "{asked:?}");
    assert!(!app.devices.named.get());
    let text = screen(&mut app, 80, 24);
    assert!(!text.contains(" Name "), "{text}");
    assert_eq!(
        frame_asks(&mut app, 200, 50),
        [(4_194_302, vec![device(4_194_302)])]
    );
}

/// The object list names its objects at 80 and at 200 columns; a name
/// with control characters shows them as spaces.
#[test]
fn the_object_list_shows_names_at_every_width() {
    let mut app = browsing(3);
    names(
        &mut app,
        7,
        vec![
            (ai(0), Some("Zone\u{1b}[2J Temp".into())),
            (ai(1), Some("x".repeat(120))),
        ],
    );
    for (w, h) in [(80, 24), (200, 50)] {
        let text = screen(&mut app, w, h);
        assert!(
            text.contains("analog-input 0  Zone [2J Temp"),
            "{w}: {text}"
        );
        assert!(text.contains("xxxx…"), "{w}: {text}");
        assert!(text.contains("analog-input 2  …"), "{w}: {text}");
    }
}

/// A passive session asks for no names.
#[test]
fn a_passive_session_asks_nothing() {
    let mut app = new_app(None);
    app.safety.passive = true;
    worker(&mut app, WorkerEvent::Passive);
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 260, 0)),
    );
    assert!(frame_asks(&mut app, 120, 40).is_empty());
}

const LAB: u32 = 100;
const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];
const RP: u8 = 12;
const RPM: u8 = 14;

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

/// The lab device's names come by ReadPropertyMultiple, a few objects a
/// request, one request outstanding at a time and each after its slot in
/// the rate cap. The device's own name came with its card.
#[tokio::test(start_paused = true)]
async fn names_are_read_through_the_gate_and_the_rate_cap() {
    let options = SessionOptions {
        max_rate: 4,
        ..SessionOptions::default()
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
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
        assert!(gap >= std::time::Duration::from_millis(250), "{gap:?}");
    }
    // Nothing is read twice.
    assert!(read_names(&mut site).await.is_empty());
    assert_eq!(site.hub.requests_to(&LAB_MAC).len(), requests.len());
    site.stop().await;
}

/// Without a rate cap, name reads still share the device's gate with an
/// object read asked at the same moment: one request outstanding at a
/// time, the object read first, and the names after it.
#[tokio::test(start_paused = true)]
async fn names_and_an_object_read_take_turns_at_the_device() {
    let options = SessionOptions {
        max_rate: 0,
        ..SessionOptions::default()
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    super::browser::open_device(&mut site).await;
    // The object read goes first, so the names wait for it.
    let mut commands = press(&mut site.app, KeyCode::Enter);
    let names = frame_commands(&mut site.app, 200, 50);
    let asked: Vec<_> = asks(&names).into_iter().flat_map(|(_, o)| o).collect();
    assert!(!asked.is_empty());
    commands.extend(names);
    site.send(commands).await;
    known(&mut site, asked).await;
    site.pump_until(|app| {
        let browser = app.browser.as_ref().unwrap();
        matches!(browser.view, crate::tui::app::browser::ViewState::Ready(_))
    })
    .await;
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}

/// A device that refuses RPM gives its names one ReadProperty each.
#[tokio::test(start_paused = true)]
async fn names_fall_back_to_read_property() {
    let server = Server {
        quirks: Quirks {
            refuse_rpm: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
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

/// The command a frame showing `objects` of the lab device would send.
fn ask(req: ReqId, objects: &[ObjectIdentifier]) -> Command {
    Command::Browse {
        req,
        device: LAB,
        read: BrowseRead::Names(objects.to_vec()),
    }
}

/// A device that won't give names: one RPM fails as a whole, each object
/// then says so alone, and the names are kept as unavailable; the worker
/// doesn't read them again even when asked.
#[tokio::test(start_paused = true)]
async fn names_a_device_wont_give_are_read_once() {
    let server = Server {
        quirks: Quirks {
            refuse_object_name: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
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
    site.run_for(std::time::Duration::from_secs(5)).await;
    assert_eq!(services_since(&site, 0).len(), 3, "not read again");
    site.stop().await;
}

/// A device that stops answering times out once, and every name still
/// asked for is given up on rather than read into more timeouts.
#[tokio::test(start_paused = true)]
async fn a_silent_device_times_out_once_for_all_its_names() {
    let server = Server {
        quirks: Quirks {
            silent: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
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
        assert_eq!(site.app.names.get(LAB, *object), Some(&Name::Unavailable));
    }
    site.stop().await;
}
