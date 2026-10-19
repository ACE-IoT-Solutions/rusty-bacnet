//! What each command does to a device: read, write or device management
//! (#972). One table serves the one-shot commands, the shell and the TUI's
//! safety levels (#964), so a command can't be guarded in one front end and
//! not in another.
//!
//! - **Read**: discovery, reads, COV subscriptions and foreign-device
//!   registration, plus the commands that never reach a device (`devices`,
//!   `capture`, `sc-check`, the shell's own `target`, `status`, `help`). A
//!   COV subscription and a registration do leave an entry on the remote
//!   side, but only to deliver what the client watches, and the TUI makes
//!   both from its read-only level; neither changes a point, a table the
//!   site configured, or how the device runs.
//! - **Write**: anything that changes a value, an object, a file, an
//!   alarm's acknowledgment, the device's clock or a BBMD table.
//! - **Device management**: communication control and reinitialize, which
//!   change whether and how the device runs at all.

use serde::Serialize;

/// What a command does to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Class {
    /// Observes only.
    Read,
    /// Changes a value, an object, a file or a table.
    Write,
    /// Changes whether or how the device runs.
    DeviceManagement,
}

impl Class {
    /// The name in messages and JSON.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::DeviceManagement => "device-management",
        }
    }

    /// True for a class that needs `--allow-write`.
    pub(crate) fn mutates(self) -> bool {
        self != Self::Read
    }
}

use Class::{DeviceManagement, Read, Write};

/// Every one-shot subcommand and shell command, aliases included, with its
/// class. A test checks this against the clap definitions and the shell's
/// command list in both directions.
const COMMANDS: &[(&str, Class)] = &[
    // Discovery and reads.
    ("discover", Read),
    ("whois", Read),
    ("find", Read),
    ("whohas", Read),
    ("read", Read),
    ("rp", Read),
    ("readm", Read),
    ("rpm", Read),
    ("read-range", Read),
    ("rr", Read),
    ("alarms", Read),
    ("file-read", Read),
    ("bdt", Read),
    ("fdt", Read),
    ("whois-router", Read),
    ("subscribe", Read),
    ("cov", Read),
    ("register", Read),
    // Nothing reaches a device, or a front end whose own commands are
    // classified one by one.
    ("devices", Read),
    ("capture", Read),
    ("sc-check", Read),
    ("shell", Read),
    ("tui", Read),
    ("target", Read),
    ("status", Read),
    ("help", Read),
    ("exit", Read),
    ("quit", Read),
    // Writes.
    ("write", Write),
    ("wp", Write),
    ("writem", Write),
    ("wpm", Write),
    ("create-object", Write),
    ("delete-object", Write),
    ("file-write", Write),
    ("ack-alarm", Write),
    ("ack", Write),
    ("time-sync", Write),
    ("ts", Write),
    // Delete-Foreign-Device-Table-Entry: the CLI names its own address,
    // but the BBMD removes whichever entry has it.
    ("unregister", Write),
    // Device management.
    ("control", DeviceManagement),
    ("dcc", DeviceManagement),
    ("reinit", DeviceManagement),
];

/// The class of a one-shot subcommand or shell command, by name or alias;
/// `None` for a word that is neither.
pub(crate) fn of(command: &str) -> Option<Class> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|&(_, class)| class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Every clap subcommand and alias, from the derived definition.
    fn clap_names() -> Vec<(String, Vec<String>)> {
        crate::parse_on_large_stack(|| {
            crate::args::Cli::command()
                .get_subcommands()
                .map(|sub| {
                    (
                        sub.get_name().to_string(),
                        sub.get_all_aliases().map(str::to_string).collect(),
                    )
                })
                .collect()
        })
    }

    /// A new subcommand or shell command can't skip classification, and an
    /// alias has its command's class.
    #[test]
    fn every_command_and_alias_has_a_class() {
        let clap = clap_names();
        assert!(clap.len() > 20, "{clap:?}");
        for (name, aliases) in &clap {
            let class = of(name).unwrap_or_else(|| panic!("subcommand {name} has no class"));
            for alias in aliases {
                assert_eq!(of(alias), Some(class), "alias {alias} of {name}");
            }
        }
        for word in crate::shell::COMMANDS {
            assert!(of(word).is_some(), "shell command {word} has no class");
        }
        // And the table names nothing that no longer exists.
        for (name, _) in COMMANDS {
            let known = clap
                .iter()
                .any(|(n, aliases)| n == name || aliases.iter().any(|a| a == name))
                || crate::shell::COMMANDS.contains(name)
                || *name == "help";
            assert!(known, "{name} is classified but is no command");
        }
    }

    /// The classes themselves, pinned: a change here is a safety change.
    #[test]
    fn the_mutating_commands_are_pinned() {
        let mutating: Vec<(&str, &str)> = COMMANDS
            .iter()
            .filter(|(_, class)| class.mutates())
            .map(|&(name, class)| (name, class.name()))
            .collect();
        assert_eq!(
            mutating,
            [
                ("write", "write"),
                ("wp", "write"),
                ("writem", "write"),
                ("wpm", "write"),
                ("create-object", "write"),
                ("delete-object", "write"),
                ("file-write", "write"),
                ("ack-alarm", "write"),
                ("ack", "write"),
                ("time-sync", "write"),
                ("ts", "write"),
                ("unregister", "write"),
                ("control", "device-management"),
                ("dcc", "device-management"),
                ("reinit", "device-management"),
            ]
        );
        assert_eq!(of("subscribe"), Some(Read));
        assert_eq!(of("register"), Some(Read));
        assert_eq!(of("no-such-command"), None);
    }
}
