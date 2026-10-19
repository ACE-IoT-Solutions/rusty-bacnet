//! Keys to [`Action`]s, per context.
//!
//! Text fields take printable characters, so `q` types a `q` in the filter or
//! the Who-Is form; Ctrl-C works everywhere.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::devices::Movement;
use super::app::network::NetworkKey;
use super::app::picker::PickerKey;
use super::app::safety::ModalKey;
use super::app::watch::Screen;
use super::app::whois::FormKey;
use super::app::{Action, App, FilterKey, Overlay};

/// Map a key press to an action, or `None` to ignore it.
pub(crate) fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        return Some(Action::CtrlC);
    }
    match &app.overlay {
        Overlay::Connection => connection(app, key),
        Overlay::Picker(_) => picker(key),
        Overlay::WhoIs(_) => form(key),
        Overlay::Help => match key.code {
            KeyCode::Esc | KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('q') => Some(Action::Quit),
            _ => None,
        },
        Overlay::Command(_) => command_line(key),
        Overlay::Safety(_) => safety(key),
        Overlay::None if app.filter_editing => filter(key),
        Overlay::None if app.screen == Screen::Watch => watch(key),
        Overlay::None if app.screen == Screen::Network => network(key),
        Overlay::None => main(key),
    }
}

/// A printable character typed without Ctrl or Alt.
fn typed(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(ch)
        }
        _ => None,
    }
}

fn movement(code: KeyCode) -> Option<Movement> {
    Some(match code {
        KeyCode::Up | KeyCode::Char('k') => Movement::Up,
        KeyCode::Down | KeyCode::Char('j') => Movement::Down,
        KeyCode::PageUp => Movement::PageUp,
        KeyCode::PageDown => Movement::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Movement::Home,
        KeyCode::End | KeyCode::Char('G') => Movement::End,
        _ => return None,
    })
}

fn main(key: KeyEvent) -> Option<Action> {
    if matches!(key.code, KeyCode::Char(_)) && typed(key).is_none() {
        return None;
    }
    if let Some(m) = movement(key.code) {
        return Some(Action::Move(m));
    }
    Some(match key.code {
        KeyCode::Esc => Action::Escape,
        KeyCode::Enter => Action::Open,
        KeyCode::Tab => Action::CyclePane { back: false },
        KeyCode::BackTab => Action::CyclePane { back: true },
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('L') => Action::ToggleLog,
        KeyCode::Char('d') => Action::OpenWhoIs,
        KeyCode::Char('p') => Action::OpenConnection,
        KeyCode::Char('c') => Action::Reconnect,
        KeyCode::Char('x') => Action::Disconnect,
        KeyCode::Char('/') => Action::StartFilter,
        KeyCode::Char('s') => Action::CycleSort,
        KeyCode::Char('S') => Action::ReverseSort,
        KeyCode::Char('a') => Action::AddWatch,
        KeyCode::Char('w') => Action::OpenWrite { relinquish: false },
        KeyCode::Char('n') => Action::OpenWrite { relinquish: true },
        KeyCode::Char('W') => Action::ToggleArm,
        KeyCode::Char('u') => Action::OpenLedger,
        KeyCode::Char(':') => Action::OpenCommandLine,
        KeyCode::Char('3') => Action::ShowScreen(Screen::Watch),
        KeyCode::Char('4') => Action::ShowScreen(Screen::Network),
        KeyCode::Char('1' | '2') => Action::ShowScreen(Screen::Main),
        _ => return None,
    })
}

/// The watch list: move, remove, the command bar and the global keys.
fn watch(key: KeyEvent) -> Option<Action> {
    if matches!(key.code, KeyCode::Char(_)) && typed(key).is_none() {
        return None;
    }
    if let Some(m) = movement(key.code) {
        return Some(Action::Move(m));
    }
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('1' | '2') => Action::ShowScreen(Screen::Main),
        KeyCode::Char('4') => Action::ShowScreen(Screen::Network),
        KeyCode::Char('x') | KeyCode::Delete => Action::RemoveWatch,
        KeyCode::Char('W') => Action::ToggleArm,
        KeyCode::Char('u') => Action::OpenLedger,
        KeyCode::Char(':') => Action::OpenCommandLine,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('L') => Action::ToggleLog,
        _ => return None,
    })
}

/// The Network screen: read, survey, switch tables and the global keys.
fn network(key: KeyEvent) -> Option<Action> {
    if matches!(key.code, KeyCode::Char(_)) && typed(key).is_none() {
        return None;
    }
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('1' | '2') => Action::ShowScreen(Screen::Main),
        KeyCode::Char('3') => Action::ShowScreen(Screen::Watch),
        KeyCode::Char('r') => Action::Network(NetworkKey::Refresh),
        KeyCode::Char('m') => Action::Network(NetworkKey::Survey),
        KeyCode::Tab => Action::Network(NetworkKey::Cycle { back: false }),
        KeyCode::BackTab => Action::Network(NetworkKey::Cycle { back: true }),
        KeyCode::Char('W') => Action::ToggleArm,
        KeyCode::Char('u') => Action::OpenLedger,
        KeyCode::Char(':') => Action::OpenCommandLine,
        KeyCode::Char('x') => Action::Disconnect,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('L') => Action::ToggleLog,
        _ => return None,
    })
}

/// Write session dialogs: forms, confirmations, the ledger and the exit
/// guard take typed characters; `q` types a `q` here rather than quitting.
fn safety(key: KeyEvent) -> Option<Action> {
    Some(Action::Modal(match key.code {
        KeyCode::Esc => ModalKey::Esc,
        KeyCode::Enter => ModalKey::Enter,
        KeyCode::Tab | KeyCode::Down => ModalKey::Next,
        KeyCode::BackTab | KeyCode::Up => ModalKey::Prev,
        KeyCode::Backspace => ModalKey::Backspace,
        _ => ModalKey::Char(typed(key)?),
    }))
}

fn command_line(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::CommandLine(FilterKey::Clear),
        KeyCode::Enter => Action::CommandLine(FilterKey::Commit),
        KeyCode::Backspace => Action::CommandLine(FilterKey::Backspace),
        _ => Action::CommandLine(FilterKey::Char(typed(key)?)),
    })
}

fn filter(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::Filter(FilterKey::Clear),
        KeyCode::Enter => Action::Filter(FilterKey::Commit),
        KeyCode::Backspace => Action::Filter(FilterKey::Backspace),
        KeyCode::Up => Action::Move(Movement::Up),
        KeyCode::Down => Action::Move(Movement::Down),
        _ => Action::Filter(FilterKey::Char(typed(key)?)),
    })
}

fn form(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc => Action::Escape,
        KeyCode::Enter => Action::Form(FormKey::Submit),
        KeyCode::Tab | KeyCode::Down => Action::Form(FormKey::Next),
        KeyCode::BackTab | KeyCode::Up => Action::Form(FormKey::Prev),
        KeyCode::Left => Action::Form(FormKey::Left),
        KeyCode::Right => Action::Form(FormKey::Right),
        KeyCode::Backspace => Action::Form(FormKey::Backspace),
        _ => Action::Form(FormKey::Char(typed(key)?)),
    })
}

fn picker(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Action::Quit,
        KeyCode::Up | KeyCode::Char('k') => Action::Picker(PickerKey::Up),
        KeyCode::Down | KeyCode::Char('j') => Action::Picker(PickerKey::Down),
        KeyCode::Enter => Action::Picker(PickerKey::Choose),
        KeyCode::Char(ch @ '1'..='9') => {
            Action::Picker(PickerKey::Number(ch as usize - '0' as usize))
        }
        _ => return None,
    })
}

fn connection(app: &App, key: KeyEvent) -> Option<Action> {
    use super::app::connection::{Key, Screen};
    let connection = app.connection.as_ref()?;
    let editing = connection.screen == Screen::Edit;
    if key.code == KeyCode::Esc {
        return Some(if editing {
            Action::Connection(Key::Discard)
        } else {
            Action::Escape
        });
    }
    if !editing && key.code == KeyCode::Char('q') {
        return Some(Action::Quit);
    }
    // Disconnect from the connection screen too; a no-op when nothing is up.
    if !editing && key.code == KeyCode::Char('x') {
        return Some(Action::Disconnect);
    }
    Some(Action::Connection(match key.code {
        KeyCode::Tab | KeyCode::Down => Key::Next,
        KeyCode::BackTab | KeyCode::Up => Key::Prev,
        KeyCode::Enter => Key::Submit,
        KeyCode::Backspace if editing => Key::Backspace,
        _ if editing => Key::Char(typed(key)?),
        KeyCode::Char('n') => Key::New,
        KeyCode::Char('s') => Key::NewSc,
        KeyCode::Char('e') => Key::Edit,
        KeyCode::Char('r') => Key::Reload,
        KeyCode::Char('p') => Key::List,
        KeyCode::Char('c') => Key::Connect,
        KeyCode::Char('a') => Key::Attest,
        _ => return None,
    }))
}
