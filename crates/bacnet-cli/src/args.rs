//! Command-line argument definitions.
//!
//! Split out of `main.rs` so that adding a flag or a subcommand does not push
//! that file into the 700-LOC cap enforced by `scripts/ci/check-file-size.sh`.
//! Dispatch stays in `main.rs`; this module is the clap surface only.

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};

use bacnet_types::primitives::BACnetTimeStamp;
use clap::builder::{OsStringValueParser, TypedValueParser};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FileReadAccess {
    Stream,
    Record,
}

#[derive(Debug, Parser)]
#[command(name = "bacnet", about = "BACnet command-line tool", version)]
pub(crate) struct Cli {
    /// Argument IDs explicitly supplied by the caller, captured before extraction.
    #[arg(skip)]
    pub(crate) explicit_args: Vec<String>,

    /// The subcommand's canonical name, captured before extraction, for the
    /// write guard's classification (`core::class`).
    #[arg(skip)]
    pub(crate) command_name: Option<String>,

    /// Let commands that change a device run: writes, object and file
    /// changes, alarm acknowledgment, time sync and unregister, and (with
    /// --yes or a typed confirmation) communication control and reinitialize.
    /// Given to the shell, it holds for the whole session. The TUI ignores
    /// it: its writes need an armed write session.
    #[arg(long, global = true)]
    pub(crate) allow_write: bool,

    /// Confirm communication control and reinitialize without typing the
    /// target; they need it when stdin is not a terminal.
    #[arg(long, global = true)]
    pub(crate) yes: bool,

    /// Network interface IP address to bind (omit to select interactively in shell mode).
    #[arg(short, long, global = true)]
    pub(crate) interface: Option<Ipv4Addr>,

    /// BACnet UDP port.
    #[arg(short, long, default_value_t = 0xBAC0, global = true)]
    pub(crate) port: u16,

    /// Broadcast address for WhoIs.
    #[arg(short, long, default_value = "255.255.255.255", global = true)]
    pub(crate) broadcast: Ipv4Addr,

    /// APDU timeout in milliseconds.
    #[arg(short, long, default_value_t = 6000, global = true)]
    pub(crate) timeout: u64,

    /// Least time in milliseconds from the latest request to a device
    /// finishing to the next being sent, or from its send while it is still
    /// outstanding (0: none, at most 3600000), so paging a log or polling
    /// leaves a slow device room for others.
    #[arg(
        long,
        default_value_t = 0,
        global = true,
        value_parser = clap::value_parser!(u64)
            .range(..=bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS)
    )]
    pub(crate) min_interval_ms: u64,

    /// Run as a foreign device registered with this BBMD (IP or IP:port), so
    /// broadcasts such as discover reach its network. A one-shot command
    /// waits for the registration first; it is renewed while the CLI runs.
    /// BACnet/IP only.
    #[arg(long, global = true, value_name = "ADDR", value_parser = crate::foreign::parse_bbmd_arg)]
    pub(crate) foreign_bbmd: Option<SocketAddrV4>,

    /// Time-to-live in seconds of a foreign-device registration: with
    /// --foreign-bbmd, register and discover --bbmd.
    #[arg(long, global = true, default_value_t = 300)]
    pub(crate) ttl: u16,

    /// Seconds between --foreign-bbmd registrations, less than --ttl
    /// (default: half the TTL). Behind NAT, renew often so the mapping the
    /// BBMD's replies use stays open.
    #[arg(long, global = true, value_name = "SECS")]
    pub(crate) renewal: Option<u64>,

    /// Use BACnet/IPv6 transport.
    #[arg(long, global = true)]
    pub(crate) ipv6: bool,

    /// IPv6 interface address to bind.
    #[arg(long, global = true)]
    pub(crate) ipv6_interface: Option<String>,

    /// Device instance whose VMAC the CLI uses on --ipv6. Without it the CLI
    /// picks a random VMAC, which needs a multicast-capable link at start
    /// (Linux loopback has none).
    #[arg(long, global = true)]
    pub(crate) device_instance: Option<u32>,

    /// Use BACnet/SC transport.
    #[arg(long, global = true)]
    pub(crate) sc: bool,

    /// SC hub WebSocket URL.
    #[arg(long, global = true)]
    pub(crate) sc_url: Option<String>,

    /// SC trusted site CA PEM file (required for SC; no system-root fallback).
    // Validate emptiness only when constructing an SC client, not for discovery
    // or a feature-disabled build. Preserve native OS path bytes.
    #[arg(long, global = true, value_name = "FILE", value_parser = OsStringValueParser::new().map(PathBuf::from))]
    pub(crate) sc_ca: Option<PathBuf>,

    /// SC TLS certificate PEM file.
    #[arg(long, global = true)]
    pub(crate) sc_cert: Option<PathBuf>,

    /// SC TLS private key PEM file.
    #[arg(long, global = true)]
    pub(crate) sc_key: Option<PathBuf>,

    /// BACnet/SC local VMAC as 12 hex digits or colon-separated bytes.
    #[arg(long, global = true, value_parser = crate::transport::parse_sc_vmac_arg)]
    pub(crate) sc_vmac: Option<[u8; 6]>,

    /// BACnet/SC device UUID as 32 hex digits or RFC 4122 hyphenated text.
    #[arg(long, global = true, value_parser = crate::transport::parse_sc_device_uuid_arg)]
    pub(crate) sc_device_uuid: Option<[u8; 16]>,

    /// Output format (table, json).
    #[arg(long, global = true)]
    pub(crate) format: Option<String>,

    /// JSON output shorthand.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

impl Cli {
    /// What `--allow-write` and `--yes` allow.
    pub(crate) fn policy(&self) -> crate::core::guard::Policy {
        crate::core::guard::Policy {
            allow_write: self.allow_write,
            yes: self.yes,
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Inspect local SC credential files offline; no hub or transport identity is needed.
    ///
    /// Requires --sc-cert; --sc-ca and --sc-key are optional. This advisory
    /// inventory does not establish TLS acceptance or change connection policy.
    ScCheck {
        /// Inspect at this signed Unix second instead of sampling the clock.
        #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
        at_unix: Option<i64>,
    },

    /// Launch interactive shell.
    Shell,

    /// Open the full-screen terminal UI (read-only until W arms writes).
    ///
    /// Opens B/IP profiles by default. Explicit transport flags without
    /// --profile retain a nonpersistent B/IP, B/IPv6 or SC connection.
    /// Writes need a profile, whose workspace holds the ledger and audit log.
    /// Needs an interactive terminal: stdin and stdout must be
    /// a TTY and TERM must not be "dumb".
    Tui {
        /// Named BACnet/IP profile to preview before connecting.
        #[arg(long, value_name = "NAME", conflicts_with = "no_profile")]
        profile: Option<String>,
        /// Transmit nothing: no client is built, so no discovery, reads,
        /// subscriptions, registrations or writes.
        #[arg(long)]
        passive: bool,
        /// Use global transport flags without reading or writing profiles.
        #[arg(long)]
        no_profile: bool,
        /// Frames per second for redraws (1-60).
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=60))]
        fps: u16,
        /// Also write log lines to this file (the in-app log pane is `L`).
        #[arg(long, value_name = "FILE")]
        log_file: Option<PathBuf>,
        /// Most confirmed requests a second the device browser sends, across
        /// all devices (0 for no cap). Each device has at most one
        /// outstanding at a time either way.
        #[arg(long, value_name = "N", default_value_t = 20)]
        max_rate: u32,
        /// Lifetime of the watch list's COV subscriptions, in seconds. They
        /// are renewed before it runs out and cancelled on exit.
        #[arg(long, value_name = "SECS", default_value_t = 300,
              value_parser = clap::value_parser!(u32).range(10..=28_800))]
        cov_lifetime: u32,
        /// Least time between two polls of a watched point a device won't
        /// report by COV, in seconds.
        #[arg(long, value_name = "SECS", default_value_t = 10,
              value_parser = clap::value_parser!(u64).range(1..=3_600))]
        poll_interval: u64,
    },

    /// Discover BACnet devices (WhoIs).
    #[command(alias = "whois")]
    Discover {
        /// Device instance range (e.g., "1000-2000").
        range: Option<String>,
        /// Seconds to wait for responses.
        #[arg(long, default_value_t = 3)]
        wait: u64,
        /// Send directed WhoIs to a specific address instead of broadcasting.
        #[arg(long, conflicts_with = "dnet")]
        target: Option<String>,
        /// Register as foreign device with a BBMD before discovering, once
        /// (for --ttl seconds). --foreign-bbmd also renews and sends the
        /// Who-Is through the BBMD.
        #[arg(long)]
        bbmd: Option<String>,
        /// Target a specific remote network number.
        #[arg(long)]
        dnet: Option<u16>,
    },

    /// Find objects by name (WhoHas).
    #[command(alias = "whohas")]
    Find {
        /// Object name to find.
        #[arg(long)]
        name: Option<String>,
        /// Seconds to wait for responses.
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },

    /// Read a property.
    #[command(alias = "rp")]
    Read {
        /// Target device (IP address or instance number).
        target: String,
        /// Object specifier (e.g., analog-input:1, ai:1).
        object: String,
        /// Property (e.g., present-value, pv).
        property: String,
    },

    /// Read multiple properties.
    #[command(alias = "rpm")]
    Readm {
        /// Target device.
        target: String,
        /// Object and property specs: "ai:1 pv,object-name ao:1 pv".
        specs: Vec<String>,
    },

    /// Write a property.
    #[command(alias = "wp")]
    Write {
        /// Target device.
        target: String,
        /// Object specifier.
        object: String,
        /// Property.
        property: String,
        /// Value to write (e.g., 72.5, true, "string", null).
        value: String,
        /// Priority (1-16).
        #[arg(long)]
        priority: Option<u8>,
    },

    /// Subscribe to COV notifications.
    #[command(alias = "cov")]
    Subscribe {
        /// Target device.
        target: String,
        /// Object specifier.
        object: String,
        /// Subscription lifetime in seconds.
        #[arg(long)]
        lifetime: Option<u32>,
        /// Use confirmed notifications.
        #[arg(long)]
        confirmed: bool,
    },

    /// Device communication control.
    #[command(alias = "dcc")]
    Control {
        /// Target device.
        target: String,
        /// Action: enable, disable, disable-initiation.
        action: String,
        /// Duration in minutes.
        #[arg(long)]
        duration: Option<u16>,
        /// Password.
        #[arg(long)]
        password: Option<String>,
    },

    /// Reinitialize device.
    Reinit {
        /// Target device.
        target: String,
        /// State: coldstart, warmstart.
        state: String,
        /// Password.
        #[arg(long)]
        password: Option<String>,
    },

    /// Get event/alarm information.
    Alarms {
        /// Target device.
        target: String,
    },

    /// Read a file from a device.
    FileRead {
        /// Target device.
        target: String,
        /// File object instance.
        file_instance: u32,
        /// BACnet file access mode.
        #[arg(long, value_enum, default_value = "stream")]
        access: FileReadAccess,
        /// Initial octet position or record index.
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Per-request octet or record window size.
        #[arg(long, default_value_t = 1024)]
        count: u32,
        /// Stream output file or required record output directory.
        #[arg(long)]
        output: Option<String>,
    },

    /// Write a file to a device.
    FileWrite {
        /// Target device.
        target: String,
        /// File object instance.
        file_instance: u32,
        /// Start position.
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Input file path.
        input: String,
    },

    /// List cached discovered devices.
    Devices,

    /// Read BBMD broadcast distribution table.
    Bdt {
        /// Target device.
        target: String,
    },

    /// Read BBMD foreign device table.
    Fdt {
        /// Target device.
        target: String,
    },

    /// Register as foreign device with BBMD, once, for --ttl seconds.
    Register {
        /// Target device.
        target: String,
    },

    /// Unregister from BBMD.
    Unregister {
        /// Target device.
        target: String,
    },

    /// Send Who-Is-Router-To-Network.
    WhoisRouter,

    /// Acknowledge an alarm.
    #[command(alias = "ack")]
    AckAlarm {
        /// Target device.
        target: String,
        /// Object specifier (e.g., ai:1).
        object: String,
        /// Event state to acknowledge (0=normal, 1=fault, etc.).
        #[arg(long)]
        state: u32,
        /// Acknowledgment source string.
        #[arg(long, default_value = "bacnet-cli")]
        source: String,
        /// Exact timestamp from the original event notification.
        #[arg(long, value_parser = crate::timestamp::parse_bacnet_timestamp)]
        timestamp: BACnetTimeStamp,
        /// Caller-selected time of acknowledgment.
        #[arg(long, value_parser = crate::timestamp::parse_bacnet_timestamp)]
        ack_time: BACnetTimeStamp,
    },

    /// Read a range of items from a list or log buffer.
    #[command(alias = "rr")]
    ReadRange {
        /// Target device.
        target: String,
        /// Object specifier (e.g., trend-log:1).
        object: String,
        /// Property (default: log-buffer).
        #[arg(default_value = "log-buffer")]
        property: String,
        #[command(flatten)]
        range: crate::commands::read_range::RangeOptions,
    },

    /// Create an object on a remote device.
    CreateObject {
        /// Target device.
        target: String,
        /// Object specifier (type:instance, e.g., av:100).
        object: String,
    },

    /// Delete an object on a remote device.
    DeleteObject {
        /// Target device.
        target: String,
        /// Object specifier (type:instance).
        object: String,
    },

    /// Synchronize time with a device.
    #[command(alias = "ts")]
    TimeSync {
        /// Target device.
        target: String,
        /// Use UTC time synchronization.
        #[arg(long)]
        utc: bool,
    },

    /// Capture and decode BACnet packets.
    Capture {
        /// Read from a pcap file instead of live capture.
        #[arg(long)]
        read: Option<PathBuf>,
        /// Save captured packets to a pcap file.
        #[arg(long)]
        save: Option<PathBuf>,
        /// Suppress decoded output (use with --save).
        #[arg(long)]
        quiet: bool,
        /// Full protocol decode (BVLC/NPDU/APDU/service details).
        #[arg(long)]
        decode: bool,
        /// Network interface name for live capture (e.g., en0, eth0).
        #[arg(long)]
        device: Option<String>,
        /// Additional BPF filter expression (appended to "udp port 47808").
        #[arg(long)]
        filter: Option<String>,
        /// Stop after capturing N packets.
        #[arg(long)]
        count: Option<u64>,
        /// Maximum bytes to capture per packet.
        #[arg(long, default_value_t = 65535)]
        snaplen: u32,
    },
}

/// Capture global argument provenance and validate cross-command requirements.
pub(crate) fn capture_sources(matches: &clap::ArgMatches) -> Result<Vec<String>, clap::Error> {
    let explicit: Vec<String> = matches
        .ids()
        .filter(|id| {
            matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine)
        })
        .map(ToString::to_string)
        .collect();
    let named_profile = matches
        .subcommand_matches("tui")
        .is_some_and(|tui| tui.get_one::<String>("profile").is_some());
    if matches.get_one::<u64>("renewal").is_some()
        && matches
            .get_one::<std::net::SocketAddrV4>("foreign_bbmd")
            .is_none()
        && !named_profile
    {
        return Err(clap::Error::raw(
            clap::error::ErrorKind::MissingRequiredArgument,
            "--renewal requires --foreign-bbmd (or a named TUI profile containing a BBMD)",
        ));
    }
    Ok(explicit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::primitives::{Date, Time};
    use clap::error::ErrorKind;

    // The derived parser's debug-build frames nearly fill a 1 MiB test thread,
    // so parse where `main` does: on a thread with a larger stack (#953).
    fn parse<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
        crate::parse_on_large_stack(move || {
            use clap::{CommandFactory, FromArgMatches};
            let mut matches = Cli::command().try_get_matches_from(args)?;
            let explicit_args = capture_sources(&matches)?;
            let command_name = matches.subcommand_name().map(str::to_owned);
            let mut cli = Cli::from_arg_matches_mut(&mut matches)?;
            cli.explicit_args = explicit_args;
            cli.command_name = command_name;
            Ok(cli)
        })
    }

    /// `--foreign-bbmd`, `--ttl` and `--renewal` reach the transport's
    /// foreign-device configuration (#938); `--ttl` also serves `register`,
    /// given before or after it.
    #[test]
    fn the_foreign_device_flags_reach_the_transport() {
        let cli = parse([
            "bacnet",
            "--foreign-bbmd",
            "192.0.2.10",
            "--renewal",
            "20",
            "devices",
            "--ttl",
            "60",
        ])
        .unwrap();
        let args = crate::transport::TransportArgs::from_cli(
            &cli,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
        )
        .unwrap();
        let foreign = args.foreign_device.unwrap();
        assert_eq!(
            (foreign.bbmd_ip, foreign.bbmd_port, foreign.ttl),
            (Ipv4Addr::new(192, 0, 2, 10), 0xBAC0, 60)
        );
        assert_eq!(
            foreign.renewal_interval,
            Some(std::time::Duration::from_secs(20))
        );

        let cli = parse(["bacnet", "register", "192.0.2.10"]).unwrap();
        assert_eq!((cli.ttl, cli.foreign_bbmd), (300, None));
        let args = crate::transport::TransportArgs::from_cli(
            &cli,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
        )
        .unwrap();
        assert!(args.foreign_device.is_none());
        let cli = parse(["bacnet", "register", "192.0.2.10", "--ttl", "45"]).unwrap();
        assert_eq!(cli.ttl, 45);
    }

    /// The guard classifies by canonical name, so an alias resolves to it;
    /// the guard flags go on either side of the subcommand (#972).
    #[test]
    fn aliases_give_the_canonical_name_and_guard_flags_are_global() {
        let cli = parse(["bacnet", "wp", "10.0.0.1", "av:1", "pv", "1"]).unwrap();
        assert_eq!(cli.command_name.as_deref(), Some("write"));
        assert_eq!(cli.policy(), crate::core::guard::Policy::default());
        let cli = parse([
            "bacnet",
            "--allow-write",
            "dcc",
            "10.0.0.1",
            "enable",
            "--yes",
        ])
        .unwrap();
        assert_eq!(cli.command_name.as_deref(), Some("control"));
        assert!(cli.allow_write && cli.yes);
        assert_eq!(parse(["bacnet"]).unwrap().command_name, None);
    }

    #[test]
    fn tui_takes_fps_and_log_file_and_global_flags() {
        let cli = parse([
            "bacnet",
            "tui",
            "--fps",
            "5",
            "--log-file",
            "tui.log",
            "-i",
            "10.0.0.5",
        ])
        .unwrap();
        assert_eq!(cli.interface, Some(Ipv4Addr::new(10, 0, 0, 5)));
        let Some(Command::Tui { fps, log_file, .. }) = cli.command else {
            panic!("expected Tui");
        };
        assert_eq!(fps, 5);
        assert_eq!(log_file, Some(PathBuf::from("tui.log")));

        let Some(Command::Tui { fps, log_file, .. }) = parse(["bacnet", "tui"]).unwrap().command
        else {
            panic!("expected Tui");
        };
        assert_eq!((fps, log_file), (20, None));
        for bad in ["0", "61"] {
            let error = parse(["bacnet", "tui", "--fps", bad]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ValueValidation);
        }
    }

    #[test]
    fn ack_alarm_and_alias_require_both_timestamp_flags() {
        for command in ["ack-alarm", "ack"] {
            let missing_both =
                parse(["bacnet", command, "127.0.0.1", "ai:1", "--state", "1"]).unwrap_err();
            assert_eq!(missing_both.kind(), ErrorKind::MissingRequiredArgument);

            let missing_ack_time = parse([
                "bacnet",
                command,
                "127.0.0.1",
                "ai:1",
                "--state",
                "1",
                "--timestamp",
                "sequence:9",
            ])
            .unwrap_err();
            assert_eq!(missing_ack_time.kind(), ErrorKind::MissingRequiredArgument);
        }
    }

    #[test]
    fn ack_alarm_clap_uses_lossless_shared_timestamp_parser() {
        let cli = parse([
            "bacnet",
            "ack",
            "127.0.0.1",
            "ai:1",
            "--state",
            "3",
            "--source",
            "operator",
            "--timestamp",
            "time:1,2,3,4",
            "--ack-time",
            "datetime:2026,9,2,3;5,6,7,8",
        ])
        .unwrap();
        let Some(Command::AckAlarm {
            state,
            source,
            timestamp,
            ack_time,
            ..
        }) = cli.command
        else {
            panic!("expected AckAlarm");
        };
        assert_eq!(state, 3);
        assert_eq!(source, "operator");
        assert_eq!(
            timestamp,
            BACnetTimeStamp::Time(Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            })
        );
        assert_eq!(
            ack_time,
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 9,
                    day: 2,
                    day_of_week: 3,
                },
                time: Time {
                    hour: 5,
                    minute: 6,
                    second: 7,
                    hundredths: 8,
                },
            }
        );
    }

    #[test]
    fn ack_alarm_clap_reports_shared_parser_error_before_execution() {
        let error = parse([
            "bacnet",
            "ack-alarm",
            "127.0.0.1",
            "ai:1",
            "--state",
            "1",
            "--timestamp",
            "time:24,0,0,0",
            "--ack-time",
            "sequence:1",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert!(error
            .to_string()
            .contains("BACnetTimeStamp hour must be 0..=23 or 255 (unspecified), got 24"));
    }
}
