//! The `:` command bar. It reads the one-shot syntax for the commands the
//! TUI runs itself:
//!
//! - `watch DEVICE OBJECT [PROPERTY]` (or `w`): add a watch row. DEVICE is
//!   a device instance from the device table, or `dnet:instance` for a
//!   routed one, in the shared target grammar; OBJECT is `ai:1` or
//!   `analog-input:1`; PROPERTY defaults to present-value.
//! - `csv on|off`: log watched values to the profile workspace, or stop.
//! - `writem DEVICE OBJECT PROPERTY VALUE[@PRIORITY] ...`: one
//!   WritePropertyMultiple to the device, any number of `OBJECT PROPERTY
//!   VALUE` groups, previewed and confirmed at T2 or above. Needs an armed
//!   write session.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::{App, FilterKey, Overlay, FLASH_FOR};
use crate::core::target::{parse_target, Target};
use crate::parse::{parse_object_specifier, parse_property, parse_value_with_priority};
use crate::tui::message::Command;
use crate::tui::safety::WriteSpec;
use crate::tui::watched::{Point, WatchCommand};

/// The usage line shown for an unknown command.
pub(crate) const USAGE: &str =
    "commands: watch DEVICE OBJECT [PROPERTY] | csv on|off | writem DEVICE OBJECT PROPERTY VALUE[@P] ...";

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    /// Watch a point.
    Watch(Point),
    /// Start or stop CSV logging.
    Csv(bool),
    /// A WritePropertyMultiple batch.
    Writem(Vec<WriteSpec>),
}

fn device(word: &str, command: &str) -> Result<u32, String> {
    match parse_target(word)? {
        Target::Instance(instance) | Target::Routed { instance, .. } => Ok(instance),
        _ => Err(format!(
            "'{word}': {command} takes a device instance (N or dnet:N) from the device table"
        )),
    }
}

fn object(word: &str) -> Result<ObjectIdentifier, String> {
    let (object_type, instance) = parse_object_specifier(word)?;
    ObjectIdentifier::new(object_type, instance).map_err(|e| format!("'{word}': {e}"))
}

/// `OBJECT PROPERTY VALUE[@P]` groups, checked with the one-shot parsers.
fn writes(device: u32, words: &[&str]) -> Result<Vec<WriteSpec>, String> {
    if words.is_empty() || !words.len().is_multiple_of(3) {
        return Err("writem DEVICE then OBJECT PROPERTY VALUE[@P] groups".into());
    }
    words
        .chunks(3)
        .map(|group| {
            let (property, index) = parse_property(group[1])?;
            let (_, priority) = parse_value_with_priority(group[2])?;
            let value = match priority {
                Some(_) => group[2].rsplit_once('@').map_or(group[2], |(v, _)| v),
                None => group[2],
            };
            Ok(WriteSpec {
                device,
                object: object(group[0])?,
                property,
                index,
                value: Some(value.to_string()),
                priority,
            })
        })
        .collect()
}

/// Parse one command line.
pub(crate) fn parse(text: &str) -> Result<Line, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        ["writem", target, rest @ ..] => Ok(Line::Writem(writes(device(target, "writem")?, rest)?)),
        [command, target, object_word, rest @ ..] if matches!(*command, "watch" | "w") => {
            let device = device(target, "watch")?;
            let object = object(object_word)?;
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
                    Ok(Line::Writem(specs)) => self.prepare(specs, true, out),
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

    #[test]
    fn writem_lines_take_object_property_value_groups() {
        let Ok(Line::Writem(specs)) = parse("writem 100 ao:1 pv 42@8 av:1 description \"x\"")
        else {
            panic!("expected writem");
        };
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].value.as_deref(), Some("42"));
        assert_eq!(specs[0].priority, Some(8));
        assert_eq!(specs[1].property, PropertyIdentifier::DESCRIPTION);
        assert_eq!(specs[1].priority, None);
        assert!(parse("writem 100 ao:1 pv").is_err());
        assert!(parse("writem 100 ao:1 pv 42@17").is_err());
        assert!(parse("writem 100").is_err());
    }
}
