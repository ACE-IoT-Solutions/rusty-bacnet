//! The `:` command bar. It reads the one-shot syntax for the commands the
//! TUI runs itself:
//!
//! - `watch DEVICE OBJECT [PROPERTY]` (or `w`): add a watch row. DEVICE is
//!   a device instance from the device table, or `dnet:instance` for a
//!   routed one, in the shared target grammar; OBJECT is `ai:1` or
//!   `analog-input:1`; PROPERTY defaults to present-value.
//! - `csv on|off`: log watched values to the profile workspace, or stop.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::{App, FilterKey, Overlay, FLASH_FOR};
use crate::core::target::{parse_target, Target};
use crate::parse::{parse_object_specifier, parse_property};
use crate::tui::message::Command;
use crate::tui::watched::{Point, WatchCommand};

/// The usage line shown for an unknown command.
pub(crate) const USAGE: &str = "commands: watch DEVICE OBJECT [PROPERTY] | csv on|off";

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    /// Watch a point.
    Watch(Point),
    /// Start or stop CSV logging.
    Csv(bool),
}

/// Parse one command line.
pub(crate) fn parse(text: &str) -> Result<Line, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        [command, device, object, rest @ ..] if matches!(*command, "watch" | "w") => {
            let device = match parse_target(device)? {
                Target::Instance(instance) | Target::Routed { instance, .. } => instance,
                _ => {
                    return Err(format!(
                    "'{device}': watch takes a device instance (N or dnet:N) from the device table"
                ))
                }
            };
            let (object_type, instance) = parse_object_specifier(object)?;
            let object = ObjectIdentifier::new(object_type, instance)
                .map_err(|e| format!("'{object}': {e}"))?;
            let property = match rest {
                [] => PropertyIdentifier::PRESENT_VALUE,
                [property] => match parse_property(property)? {
                    (property, None) => property,
                    (_, Some(_)) => return Err("the watch list takes no array index".into()),
                },
                _ => return Err(format!("too many words; {USAGE}")),
            };
            Ok(Line::Watch(Point {
                device,
                object,
                property,
            }))
        }
        ["csv", "on"] => Ok(Line::Csv(true)),
        ["csv", "off"] => Ok(Line::Csv(false)),
        [] => Err(USAGE.into()),
        _ => Err(format!("unknown command '{}'; {USAGE}", text.trim())),
    }
}

impl App {
    /// `:`: open the command bar.
    pub(super) fn open_command_line(&mut self) {
        self.overlay = Overlay::Command(String::new());
    }

    /// A key on the command bar.
    pub(super) fn command_line_key(&mut self, key: FilterKey, out: &mut Vec<Command>) {
        let Overlay::Command(text) = &mut self.overlay else {
            return;
        };
        match key {
            FilterKey::Char(ch) => text.push(ch),
            FilterKey::Backspace => {
                text.pop();
            }
            FilterKey::Clear => self.overlay = Overlay::None,
            FilterKey::Commit => {
                let line = std::mem::take(text);
                self.overlay = Overlay::None;
                match parse(&line) {
                    Ok(Line::Watch(point)) => self.watch_point(point, out),
                    Ok(Line::Csv(on)) => out.push(Command::Watch(WatchCommand::Csv(on))),
                    Err(error) => self.flash(error, FLASH_FOR),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;

    #[test]
    fn watch_lines_name_a_device_an_object_and_maybe_a_property() {
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(
            parse("watch 100 ai:1"),
            Ok(Line::Watch(Point {
                device: 100,
                object: ai1,
                property: PropertyIdentifier::PRESENT_VALUE
            }))
        );
        assert_eq!(
            parse("w 2:100 analog-input:1 sf"),
            Ok(Line::Watch(Point {
                device: 100,
                object: ai1,
                property: PropertyIdentifier::STATUS_FLAGS
            }))
        );
        assert_eq!(parse("csv on"), Ok(Line::Csv(true)));
        assert!(parse("watch 10.0.0.5 ai:1")
            .unwrap_err()
            .contains("device instance"));
        assert!(parse("watch 100 ai:1 object-list[2]").is_err());
        assert!(parse("watch 100 ai:x").is_err());
        assert!(parse("read 100 ai:1")
            .unwrap_err()
            .contains("unknown command"));
    }
}
