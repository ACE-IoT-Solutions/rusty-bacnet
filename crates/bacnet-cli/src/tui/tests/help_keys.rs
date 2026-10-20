//! The screen strip, the help overlay and the command bar's hints (#1895):
//! the strip lists only the screens that apply to the session, help names
//! every key the key map takes on the screen shown and scrolls at 80x24,
//! and the command bar shows the syntax of what is being typed.

use std::collections::BTreeSet;

use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::tui::app::watch::Screen;
use crate::tui::app::Overlay;
use crate::tui::help::{screens, sections};
use crate::tui::monitor::MonitorPlan;

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 50)];

fn app_on(transport: &'static str, style: AddressStyle) -> App {
    App::new(
        AppConfig {
            transport,
            style,
            color: false,
            picker: None,
            join: None,
            passive: false,
            now: t0(),
        },
        LogRing::new(100),
    )
}

fn connected(transport: &'static str, style: AddressStyle, local: &str) -> App {
    let mut app = app_on(transport, style);
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: local.into(),
        },
    );
    app
}

/// The status bar of a `width` x `height` frame.
fn status(app: &mut App, width: u16, height: u16) -> String {
    let text = screen(app, width, height);
    text.lines().next().unwrap().trim_matches('"').to_string()
}

fn keys(app: &App) -> Vec<char> {
    screens(app).iter().map(|tab| tab.key).collect()
}

#[test]
fn the_strip_lists_only_the_screens_that_apply() {
    let mut out = String::new();
    let mut bip = connected_app();
    let mut sc = connected("SC", AddressStyle::Hex, "22:10:20:30:40:50");
    let mut mstp = connected("MSTP", AddressStyle::Mstp, "mstp:3");
    assert_eq!(keys(&bip), ['1', '2', '3', '4', '7']);
    assert_eq!(keys(&sc), ['1', '2', '3', '5', '7']);
    assert_eq!(keys(&mstp), ['1', '2', '3', '6', '7']);
    for (name, app) in [("B/IP", &mut bip), ("SC", &mut sc), ("MS/TP", &mut mstp)] {
        press(app, KeyCode::Char('3'));
        for (w, h) in SIZES {
            let bar = status(app, w, h);
            assert!(bar.contains("[3 Watch]"), "{name} {w}: {bar}");
            out.push_str(&format!("{name} {w}x{h}\n{bar}\n"));
        }
    }
    // The full names where there is room.
    let bar = status(&mut bip, 200, 50);
    assert!(
        bar.contains("1 Devices 2 Browse [3 Watch] 4 Network 7 Capture"),
        "{bar}"
    );
    for absent in ["5 SC", "6 MS/TP"] {
        assert!(!bar.contains(absent), "{bar}");
    }
    assert_snapshot!("strip_by_session", out);
}

#[test]
fn the_strip_names_the_screen_shown_even_where_it_does_not_apply() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(keys(&app), ['1', '2', '3', '4', '5', '7']);
    assert!(status(&mut app, 80, 24).contains("[5 SC]"));
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(keys(&app), ['1', '2', '3', '4', '7']);
}

#[test]
fn the_monitor_and_an_open_browser_have_their_place_in_the_strip() {
    let mut app = app_on("MSTP", AddressStyle::Mstp);
    app.start_monitor(&MonitorPlan::new("/dev/ttyUSB7".into(), 38400, 1), true);
    assert!(keys(&app).contains(&'6'));
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.1.21:47808", 260, 2)),
    );
    press(&mut app, KeyCode::Enter);
    assert!(app.browser.is_some());
    assert!(status(&mut app, 80, 24).contains("[2 Browse]"));
}

/// A key as help names it.
fn token(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "Shift-Tab".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Delete => "Delete".into(),
        other => format!("{other:?}"),
    }
}

/// Every key worth pressing: the printable ASCII characters and the
/// named keys.
fn every_key() -> Vec<KeyCode> {
    let mut codes: Vec<KeyCode> = (0x20u8..0x7f).map(|b| KeyCode::Char(b as char)).collect();
    codes.extend([
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Insert,
        KeyCode::F(1),
    ]);
    codes
}

/// The keys named in the help shown for `app`, outside the command list.
fn named(app: &App) -> BTreeSet<String> {
    sections(app)
        .into_iter()
        .filter(|section| !section.title.starts_with("Commands"))
        .flat_map(|section| section.rows)
        .flat_map(|(keys, _)| {
            keys.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Each key the key map takes on a screen is named in that screen's help.
#[test]
fn help_names_every_key_each_screen_takes() {
    let mut contexts: Vec<(&str, App)> = Vec::new();
    contexts.push(("devices", connected_app()));
    let mut browsing = connected_app();
    worker(
        &mut browsing,
        WorkerEvent::Discovered(row(1001, "10.0.1.21:47808", 260, 2)),
    );
    press(&mut browsing, KeyCode::Enter);
    assert!(browsing.browser.is_some());
    contexts.push(("browser", browsing));
    for (name, key) in [
        ("watch", '3'),
        ("network", '4'),
        ("sc", '5'),
        ("mstp", '6'),
        ("capture", '7'),
    ] {
        let mut app = connected_app();
        press(&mut app, KeyCode::Char(key));
        contexts.push((name, app));
    }
    let mut tree = connected_app();
    press(&mut tree, KeyCode::Char('7'));
    press(&mut tree, KeyCode::Tab);
    contexts.push(("capture tree", tree));
    for (name, app) in &contexts {
        assert_eq!(app.overlay, Overlay::None, "{name}");
        let named = named(app);
        let missing: Vec<String> = every_key()
            .into_iter()
            .filter(|code| map_key(app, KeyEvent::new(*code, KeyModifiers::NONE)).is_some())
            .map(token)
            .filter(|token| !named.contains(token))
            .collect();
        assert!(
            missing.is_empty(),
            "{name}: help leaves out {missing:?}; it names {named:?}"
        );
    }
}

/// Help for `screen` at 80x24, from the top and scrolled to the end.
fn help_frames(app: &mut App) -> (String, String) {
    press(app, KeyCode::Char('?'));
    assert_eq!(app.overlay, Overlay::Help);
    let top = screen(app, 80, 24);
    press(app, KeyCode::End);
    let end = screen(app, 80, 24);
    (top, end)
}

#[test]
fn help_scrolls_at_80x24() {
    let mut app = connected_app();
    let (top, end) = help_frames(&mut app);
    assert!(top.contains(" Global"), "{top}");
    assert!(top.contains("1-20 of"), "{top}");
    assert!(!top.contains("Commands (:)"), "{top}");
    assert!(end.contains("Commands (:)"), "{end}");
    assert!(end.contains("bbmd [ADDRESS]"), "{end}");
    assert!(end.contains("mstp listen SECS"), "{end}");
    assert_snapshot!("help_devices_80x24_top", top);
    assert_snapshot!("help_devices_80x24_end", end);
    // End leaves the view on the last page; Up moves from there.
    let last = app.help_scroll.get();
    press(&mut app, KeyCode::Up);
    assert_eq!(app.help_scroll.get(), last - 1);
    press(&mut app, KeyCode::Home);
    assert_eq!(app.help_scroll.get(), 0);
    // Closing and opening again starts at the top.
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.help_scroll.get(), 0);

    let mut app = connected_app();
    press(&mut app, KeyCode::Char('4'));
    let (top, end) = help_frames(&mut app);
    assert!(top.contains("Network (4)"), "{top}");
    assert!(all_rows(&app).contains("from 160 columns"));
    assert!(!top.contains("Devices (1)"), "{top}");
    assert_snapshot!("help_network_80x24_top", top);
    assert_snapshot!("help_network_80x24_end", end);
}

/// Every row of the help for `app`, as text.
fn all_rows(app: &App) -> String {
    sections(app)
        .into_iter()
        .flat_map(|s| s.rows)
        .map(|(keys, what)| format!("{keys} {what}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn help_names_what_the_issue_found_missing() {
    let mut app = connected_app();
    let all = all_rows;
    let devices = all(&app);
    for text in [
        "writem",
        "bbmd",
        "mstp rotation",
        "csv on|off",
        "2 is Browse",
    ] {
        assert!(devices.contains(text), "{text}");
    }
    press(&mut app, KeyCode::Char('5'));
    assert!(all(&app).contains("my credentials"));
    press(&mut app, KeyCode::Char('6'));
    let mstp = all(&app);
    for text in ["J Monitor", "+ = -", "Space"] {
        assert!(mstp.contains(text), "{text}: {mstp}");
    }
}

/// The footer while typing a command.
fn typing(app: &mut App, text: &str) -> String {
    press(app, KeyCode::Char(':'));
    type_text(app, text);
    let frame = screen(app, 80, 24);
    press(app, KeyCode::Esc);
    frame.lines().last().unwrap().trim_matches('"').to_string()
}

#[test]
fn the_command_bar_shows_the_syntax_being_typed() {
    let mut app = connected_app();
    let mut out = String::new();
    for text in ["", "w", "wa", "watch 100 ", "mstp", "bb", "zz"] {
        let footer = typing(&mut app, text);
        assert!(
            footer.trim_end().ends_with("Enter run  Esc close"),
            "{footer}"
        );
        out.push_str(&format!("{text:?}\n{footer}\n"));
    }
    assert!(typing(&mut app, "wa").contains("watch DEVICE OBJECT [PROPERTY]"));
    assert!(!typing(&mut app, "wa").contains("writem"));
    let w = typing(&mut app, "w");
    assert!(
        w.contains("watch DEVICE") && w.contains("writem DEVICE"),
        "{w}"
    );
    assert!(typing(&mut app, "mstp").contains("mstp rotation|hold MS | mstp listen SECS"));
    assert!(typing(&mut app, "zz").contains("no such command"));
    assert_snapshot!("command_bar_hints", out);
}

#[test]
fn help_opens_on_every_screen_without_a_session() {
    let mut app = new_app(None);
    for screen in [
        Screen::Main,
        Screen::Watch,
        Screen::Network,
        Screen::Sc,
        Screen::Mstp,
        Screen::Capture,
    ] {
        app.screen = screen;
        assert!(sections(&app).len() >= 3, "{screen:?}");
    }
}
