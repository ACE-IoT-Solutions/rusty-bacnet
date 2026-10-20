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
//! - `bbmd [ADDRESS]`: show and read that BBMD's tables on the Network screen
//!   (`ip` or `ip:port`, port 47808 by default); `bbmd` alone goes back to
//!   the tables of the BBMD the session registers with.
//! - `find NAME`: broadcast a Who-Has for the object named NAME, the rest
//!   of the line as typed, and list the answers below the device table;
//!   `find` alone closes the list.
//! - `mstp rotation MS`, `mstp hold MS`, `mstp listen SECS`: the MS/TP
//!   monitor's thresholds, and how long its join wizard listens.

use std::time::Duration;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::monitor::Setting;
use super::{App, FilterKey, Overlay, FLASH_FOR};
use crate::core::target::{parse_target, Target};
use crate::parse::{parse_object_specifier, parse_property, parse_value_with_priority};
use crate::tui::message::Command;
use crate::tui::safety::WriteSpec;
use crate::tui::watched::{Point, WatchCommand};

/// One command: its name, a short alias, its forms and what it does. The
/// parser finds the command here, and the command bar's hint, the help
/// overlay and the usage line all read the same rows (#1895), so none can
/// name a command the parser doesn't take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Syntax {
    /// The first word.
    pub(crate) name: &'static str,
    /// Another first word for it, if any.
    pub(crate) alias: Option<&'static str>,
    /// Each form, as typed.
    pub(crate) usage: &'static [&'static str],
    /// What it does, in a few words.
    pub(crate) what: &'static str,
}

/// Every command the bar takes.
pub(crate) const COMMANDS: &[Syntax] = &[
    Syntax {
        name: "watch",
        alias: Some("w"),
        usage: &["watch DEVICE OBJECT [PROPERTY]"],
        what: "Watch a property (present-value by default); w for short",
    },
    Syntax {
        name: "csv",
        alias: None,
        usage: &["csv on|off"],
        what: "Log the watched values to the workspace, or stop",
    },
    Syntax {
        name: "writem",
        alias: None,
        usage: &["writem DEVICE OBJECT PROPERTY VALUE[@P] ..."],
        what: "One WritePropertyMultiple to a device, previewed first",
    },
    Syntax {
        name: "bbmd",
        alias: None,
        usage: &["bbmd [ADDRESS]"],
        what: "Another BBMD's tables; alone, back to our own BBMD's",
    },
    Syntax {
        name: "find",
        alias: None,
        usage: &["find NAME"],
        what: "Who-Has by object name; alone, close the list",
    },
    Syntax {
        name: "mstp",
        alias: None,
        usage: &["mstp rotation|hold MS", "mstp listen SECS"],
        what: "The MS/TP monitor's thresholds; how long a safe join listens",
    },
];

/// The command whose name or alias is `word`.
fn command(word: &str) -> Option<&'static Syntax> {
    COMMANDS
        .iter()
        .find(|syntax| syntax.name == word || syntax.alias == Some(word))
}

/// The usage line shown for an unknown command.
pub(crate) fn usage() -> String {
    let forms: Vec<&str> = COMMANDS
        .iter()
        .flat_map(|s| s.usage.iter().copied())
        .collect();
    format!("commands: {}", forms.join(" | "))
}

/// The commands that match what is typed so far, for the hint beside the
/// bar: the one named by a complete first word, or every one whose name
/// starts with a partial one (all of them before anything is typed).
pub(crate) fn matching(text: &str) -> Vec<&'static Syntax> {
    let first = text.split_whitespace().next().unwrap_or_default();
    let typed_on = text.trim_start().len() > first.len();
    if let Some(syntax) = command(first).filter(|_| typed_on || first.len() > 1) {
        return vec![syntax];
    }
    if typed_on {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|syntax| syntax.name.starts_with(first))
        .collect()
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    /// Watch a point.
    Watch(Point),
    /// Start or stop CSV logging.
    Csv(bool),
    /// A WritePropertyMultiple batch.
    Writem(Vec<WriteSpec>),
    /// Show a BBMD's tables.
    Bbmd(Option<std::net::SocketAddrV4>),
    /// Search for an object by name; empty closes the search.
    Find(String),
    /// An MS/TP monitor setting.
    Mstp(Setting),
}

/// `mstp rotation|hold MS` or `mstp listen SECS`, each at least 1.
fn mstp_setting(which: &str, value: &str) -> Result<Setting, String> {
    let number = value
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=3_600_000).contains(n))
        .ok_or_else(|| format!("'{value}': a whole number from 1"))?;
    match which {
        "rotation" => Ok(Setting::Rotation(Duration::from_millis(number))),
        "hold" => Ok(Setting::Hold(Duration::from_millis(number))),
        "listen" if number <= 3_600 => Ok(Setting::Listen(Duration::from_secs(number))),
        "listen" => Err("mstp listen takes 1 to 3600 seconds".into()),
        _ => Err(format!(
            "'{which}': mstp rotation|hold MS, or mstp listen SECS"
        )),
    }
}

/// A device instance, with the network when typed as `dnet:instance`.
fn device(word: &str, command: &str) -> Result<(u32, Option<u16>), String> {
    match parse_target(word)? {
        Target::Instance(instance) => Ok((instance, None)),
        Target::Routed { dnet, instance } => Ok((instance, Some(dnet))),
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

/// Parse one command line: its first word picks the row of [`COMMANDS`],
/// and the rest is read by that row's form.
pub(crate) fn parse(text: &str) -> Result<Line, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some((first, args)) = words.split_first() else {
        return Err(usage());
    };
    let Some(syntax) = command(first) else {
        return Err(format!("unknown command '{}'; {}", text.trim(), usage()));
    };
    let wrong = || format!("usage: {}", syntax.usage.join(" | "));
    match (syntax.name, args) {
        ("writem", [target, rest @ ..]) => {
            Ok(Line::Writem(writes(device(target, "writem")?.0, rest)?))
        }
        ("watch", [target, object_word, rest @ ..]) => {
            let (device, network) = device(target, "watch")?;
            let object = object(object_word)?;
            let property = match rest {
                [] => PropertyIdentifier::PRESENT_VALUE,
                [property] => match parse_property(property)? {
                    (property, None) => property,
                    (_, Some(_)) => return Err("the watch list takes no array index".into()),
                },
                _ => return Err(format!("too many words; {}", wrong())),
            };
            Ok(Line::Watch(Point {
                device,
                network,
                object,
                property,
            }))
        }
        ("csv", ["on"]) => Ok(Line::Csv(true)),
        ("csv", ["off"]) => Ok(Line::Csv(false)),
        ("bbmd", []) => Ok(Line::Bbmd(None)),
        ("bbmd", [address]) => Ok(Line::Bbmd(Some(crate::foreign::parse_bbmd_arg(address)?))),
        ("mstp", [which, value]) => Ok(Line::Mstp(mstp_setting(which, value)?)),
        ("find", _) => {
            // A name may hold several spaces in a row: keep it as typed.
            let rest = text.trim_start().strip_prefix("find").unwrap_or_default();
            Ok(Line::Find(rest.trim().to_string()))
        }
        _ => Err(wrong()),
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
                    Ok(Line::Bbmd(bbmd)) => self.show_bbmd(bbmd, out),
                    Ok(Line::Find(name)) => self.find(name, out),
                    Ok(Line::Mstp(setting)) => self.mstp_setting(setting),
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
                network: None,
                object: ai1,
                property: PropertyIdentifier::PRESENT_VALUE
            }))
        );
        assert_eq!(
            parse("w 2:100 analog-input:1 sf"),
            Ok(Line::Watch(Point {
                device: 100,
                network: Some(2),
                object: ai1,
                property: PropertyIdentifier::STATUS_FLAGS
            }))
        );
        assert_eq!(parse("csv on"), Ok(Line::Csv(true)));
        assert_eq!(
            parse("bbmd 10.0.0.1"),
            Ok(Line::Bbmd(Some("10.0.0.1:47808".parse().unwrap())))
        );
        assert!(parse("bbmd 10.0.0").is_err());
        assert_eq!(parse("bbmd"), Ok(Line::Bbmd(None)));
        assert!(parse("watch 10.0.0.5 ai:1")
            .unwrap_err()
            .contains("device instance"));
        assert!(parse("watch 100 ai:1 object-list[2]").is_err());
        assert!(parse("watch 100 ai:x").is_err());
        assert_eq!(
            parse("find  Supply  Air Temp "),
            Ok(Line::Find("Supply  Air Temp".into()))
        );
        assert_eq!(parse("find"), Ok(Line::Find(String::new())));
        let ms = Duration::from_millis;
        assert_eq!(
            parse("mstp rotation 1500"),
            Ok(Line::Mstp(Setting::Rotation(ms(1500))))
        );
        assert_eq!(
            parse("mstp hold 250"),
            Ok(Line::Mstp(Setting::Hold(ms(250))))
        );
        assert_eq!(
            parse("mstp listen 30"),
            Ok(Line::Mstp(Setting::Listen(Duration::from_secs(30))))
        );
        assert!(parse("mstp hold 0").is_err());
        assert!(parse("mstp listen 4000").is_err());
        assert!(parse("mstp poll 5").is_err());
        assert!(parse("finder x").unwrap_err().contains("unknown command"));
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

    /// The table row a parsed line came from. A new `Line` fails to
    /// compile here until it names its row.
    fn row_of(line: &Line) -> &'static str {
        match line {
            Line::Watch(_) => "watch",
            Line::Csv(_) => "csv",
            Line::Writem(_) => "writem",
            Line::Bbmd(_) => "bbmd",
            Line::Find(_) => "find",
            Line::Mstp(_) => "mstp",
        }
    }

    /// Every parsed command has a row, so a hint and a help line; every
    /// row parses; and the hint for a line is its own row's.
    #[test]
    fn every_command_has_its_hint_from_the_table_the_parser_reads() {
        let samples = [
            "watch 100 ai:1",
            "w 100 ai:1 pv",
            "csv on",
            "writem 100 ao:1 pv 42@8",
            "bbmd",
            "bbmd 10.0.0.1",
            "find Supply Temp",
            "mstp hold 250",
            "mstp listen 30",
        ];
        let mut rows = std::collections::BTreeSet::new();
        for sample in samples {
            let line = parse(sample).unwrap_or_else(|e| panic!("{sample}: {e}"));
            let row = row_of(&line);
            assert!(command(row).is_some(), "{row} has a row");
            let hint = matching(sample);
            assert_eq!(hint.len(), 1, "{sample}");
            assert_eq!(hint[0].name, row, "{sample}");
            rows.insert(row);
        }
        let names: std::collections::BTreeSet<&str> = COMMANDS.iter().map(|s| s.name).collect();
        assert_eq!(rows, names, "every row is parsed by a sample");
        for syntax in COMMANDS {
            for form in syntax.usage {
                assert_eq!(form.split_whitespace().next(), Some(syntax.name), "{form}");
            }
            assert!(usage().contains(syntax.usage[0]));
        }
        // A wrong form names its command's usage.
        assert_eq!(parse("csv maybe"), Err("usage: csv on|off".into()));
    }

    #[test]
    fn hints_follow_what_is_typed() {
        let names = |text: &str| -> Vec<&str> { matching(text).iter().map(|s| s.name).collect() };
        assert_eq!(names("").len(), COMMANDS.len());
        assert_eq!(names("w"), ["watch", "writem"]);
        assert_eq!(names("w "), ["watch"]);
        assert_eq!(names("wr"), ["writem"]);
        assert_eq!(names("mstp hold"), ["mstp"]);
        assert!(names("zz").is_empty());
        assert!(names("zz 1").is_empty());
    }
}
