//! MS/TP in the CLI (#969): the `--mstp` flags and their checks, the client
//! that joins the trunk as a master station, and `mstp-ports`.
//!
//! The flags are always there, so a build without the `serial` feature can
//! say how to get it. Joining a trunk is a deliberate act: `--mstp` is the
//! intent, and `--mstp-mac` has no default, since two masters with one MAC
//! break the token ring for every station on it.

use std::io::Write;

use bacnet_client::client::BACnetClient;
use bacnet_transport::mstp::{
    MstpConfig, MstpDiagnostics, MstpExecutionMode, MstpTransport, SerialPort,
};
use bacnet_types::error::Error;

use crate::output::OutputFormat;

/// The baud rates an MS/TP port may run at (Clause 9.2.3).
pub(crate) const BAUD_RATES: [u32; 6] = [9_600, 19_200, 38_400, 57_600, 76_800, 115_200];

/// `--mstp-baud`'s default, the Python bindings' too.
pub(crate) const DEFAULT_BAUD: u32 = 38_400;

/// The highest MAC a master station may have, and Max_Master's limit.
pub(crate) const MAX_MASTER_MAC: u8 = 127;

/// `--mstp-baud`: one of [`BAUD_RATES`].
pub(crate) fn parse_baud(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|rate| BAUD_RATES.contains(rate))
        .ok_or_else(|| {
            "must be one of 9600, 19200, 38400, 57600, 76800 or 115200 (Clause 9.2.3)".into()
        })
}

/// `--mstp-mac`: a master station's MAC, 0 to 127.
pub(crate) fn parse_master_mac(text: &str) -> Result<u8, String> {
    parse_up_to_127(text, "a master station's MAC")
}

/// `--mstp-max-master`: Max_Master, 0 to 127.
pub(crate) fn parse_max_master(text: &str) -> Result<u8, String> {
    parse_up_to_127(text, "Max_Master")
}

fn parse_up_to_127(text: &str, what: &str) -> Result<u8, String> {
    text.parse::<u8>()
        .ok()
        .filter(|value| *value <= MAX_MASTER_MAC)
        .ok_or_else(|| format!("{what} must be 0 to 127"))
}

/// `--mstp-max-info-frames`: frames per token use, 1 to 255.
pub(crate) fn parse_max_info_frames(text: &str) -> Result<u8, String> {
    text.parse::<u8>()
        .ok()
        .filter(|value| *value >= 1)
        .ok_or_else(|| "must be 1 to 255".into())
}

/// The global flags of the other transports, which MS/TP has no use for:
/// B/IP's interface, port, broadcast and foreign-device settings, B/IPv6's
/// and BACnet/SC's.
const OTHER_TRANSPORT_FLAGS: [&str; 16] = [
    "interface",
    "port",
    "broadcast",
    "foreign_bbmd",
    "ttl",
    "renewal",
    "ipv6",
    "ipv6_interface",
    "device_instance",
    "sc",
    "sc_url",
    "sc_ca",
    "sc_cert",
    "sc_key",
    "sc_vmac",
    "sc_device_uuid",
];

/// Why `--mstp` needs `--mstp-mac`.
const NEEDS_MAC: &str =
    "--mstp needs --mstp-mac (0 to 127): a station address no other device on the trunk uses";

/// Why a MAC above Max_Master is refused.
const ABOVE_MAX_MASTER: &str = "no station would ever pass this one the token";

/// The flag checks that take two flags, at parse time, so they fail as a
/// usage error (exit status 2, nothing on stdout), on whichever side of the
/// subcommand each flag is: `--mstp` needs a port name, `--mstp-mac`, and no
/// other transport's flags; the MAC may not be above `--mstp-max-master`;
/// and the other MS/TP flags need `--mstp`.
pub(crate) fn check_matches(matches: &clap::ArgMatches) -> Result<(), clap::Error> {
    use clap::error::ErrorKind;
    if matches.get_one::<String>("mstp").is_none() {
        // Clap's `requires` misses a global flag given after the subcommand.
        let given = [
            "mstp_baud",
            "mstp_mac",
            "mstp_max_master",
            "mstp_max_info_frames",
        ]
        .into_iter()
        .find(|id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine));
        return match given {
            Some(id) => Err(clap::Error::raw(
                ErrorKind::MissingRequiredArgument,
                format!("--{} requires --mstp PORT\n", id.replace('_', "-")),
            )),
            None => Ok(()),
        };
    }
    let given = |id: &str| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine);
    // Clap's `conflicts_with` misses these too when they sit on the other
    // side of the subcommand, and MS/TP would ignore them without a word.
    if let Some(id) = OTHER_TRANSPORT_FLAGS.iter().find(|id| given(id)) {
        return Err(clap::Error::raw(
            ErrorKind::ArgumentConflict,
            format!(
                "--{} cannot be used with --mstp: it belongs to another transport\n",
                id.replace('_', "-")
            ),
        ));
    }
    if matches
        .get_one::<String>("mstp")
        .is_some_and(|port| port.trim().is_empty())
    {
        return Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            "--mstp needs a serial port name (see `bacnet mstp-ports`)\n",
        ));
    }
    let Some(&mac) = matches.get_one::<u8>("mstp_mac") else {
        return Err(clap::Error::raw(
            ErrorKind::MissingRequiredArgument,
            format!("{NEEDS_MAC}\n"),
        ));
    };
    let max_master = matches
        .get_one::<u8>("mstp_max_master")
        .copied()
        .unwrap_or(MAX_MASTER_MAC);
    if mac > max_master {
        return Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            format!(
                "--mstp-mac {mac} is above --mstp-max-master {max_master}: {ABOVE_MAX_MASTER}\n"
            ),
        ));
    }
    Ok(())
}

/// The MS/TP settings of one run: from the flags, or from a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MstpArgs {
    /// The serial port's name, as `mstp-ports` lists it.
    pub(crate) port: String,
    /// One of [`BAUD_RATES`].
    pub(crate) baud: u32,
    /// This station's MAC, 0 to 127.
    pub(crate) mac: u8,
    /// Max_Master, at least `mac`.
    pub(crate) max_master: u8,
    /// Max_Info_Frames, 1 to 255.
    pub(crate) max_info_frames: u8,
}

impl MstpArgs {
    /// The flags' settings, `None` without `--mstp`. Clap has checked them
    /// already, each flag's range and then [`check_matches`].
    pub(crate) fn from_cli(cli: &crate::args::Cli) -> Result<Option<Self>, String> {
        let Some(port) = cli.mstp.clone() else {
            return Ok(None);
        };
        let mac = cli.mstp_mac.ok_or(NEEDS_MAC)?;
        let args = Self {
            port,
            baud: cli.mstp_baud,
            mac,
            max_master: cli.mstp_max_master,
            max_info_frames: cli.mstp_max_info_frames,
        };
        args.validate()?;
        Ok(Some(args))
    }

    /// The checks the flags' parsers make, for settings from a profile, and
    /// the cross-field ones: the port named, and the MAC within Max_Master.
    /// Messages name the profile's keys.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.port.trim().is_empty() {
            return Err("the MS/TP serial port is required (see `bacnet mstp-ports`)".into());
        }
        let checks = [
            parse_baud(&self.baud.to_string()).map(drop),
            parse_master_mac(&self.mac.to_string()).map(drop),
            parse_max_master(&self.max_master.to_string()).map(drop),
            parse_max_info_frames(&self.max_info_frames.to_string()).map(drop),
        ];
        for (check, flag) in
            checks
                .into_iter()
                .zip(["baud", "mac", "max_master", "max_info_frames"])
        {
            check.map_err(|error| format!("{flag} {error}"))?;
        }
        if self.mac > self.max_master {
            return Err(format!(
                "mac {} is above max_master {}: {ABOVE_MAX_MASTER}",
                self.mac, self.max_master
            ));
        }
        Ok(())
    }

    /// The transport's configuration; the timing values keep their defaults.
    #[cfg_attr(not(feature = "serial"), allow(dead_code))]
    pub(crate) fn config(&self) -> MstpConfig {
        MstpConfig {
            this_station: self.mac,
            max_master: self.max_master,
            max_info_frames: self.max_info_frames,
            baud_rate: self.baud,
            ..MstpConfig::default()
        }
    }
}

#[cfg_attr(not(feature = "serial"), allow(dead_code))]
/// Build a client that joins the trunk on `serial` as station `args.mac`.
///
/// The MAC loop runs on a dedicated thread, so a busy application runtime
/// doesn't delay token handling. The diagnostics handle is taken before
/// the client owns the transport; it also carries the node's state.
pub(crate) async fn build_client_on<S: SerialPort>(
    serial: S,
    args: &MstpArgs,
    timeout_ms: u64,
    min_interval_ms: u64,
) -> Result<(BACnetClient<MstpTransport<S>>, MstpDiagnostics), Error> {
    let config = args.config();
    config.validate()?;
    let max_apdu = config.max_apdu_length();
    let transport =
        MstpTransport::new(serial, config).with_execution_mode(MstpExecutionMode::DedicatedThread);
    let diagnostics = transport.diagnostics();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .max_apdu_length(max_apdu)
        .apdu_timeout_ms(timeout_ms)
        .min_request_interval_ms(min_interval_ms)
        .build()
        .await?;
    Ok((client, diagnostics))
}

/// The serial port type the CLI opens.
#[cfg(feature = "serial")]
pub(crate) type Serial = bacnet_transport::mstp_serial::TokioSerialPort;

/// Open `args.mstp`'s serial port, after checking its settings, and join
/// the trunk there.
#[cfg(feature = "serial")]
pub(crate) async fn build_client(
    args: &crate::transport::TransportArgs,
) -> Result<(BACnetClient<MstpTransport<Serial>>, MstpDiagnostics), Error> {
    use bacnet_transport::mstp_serial::SerialConfig;
    let mstp = args
        .mstp
        .as_ref()
        .ok_or_else(|| Error::Encoding("--mstp is required for MS/TP".into()))?;
    mstp.validate().map_err(Error::Encoding)?;
    mstp.config().validate()?;
    let serial = Serial::open(&SerialConfig {
        port_name: mstp.port.clone(),
        baud_rate: mstp.baud,
    })
    .map_err(|error| match error {
        Error::Encoding(message) => Error::Encoding(format!("{}: {message}", mstp.port)),
        other => other,
    })?;
    build_client_on(serial, mstp, args.timeout_ms, args.min_interval_ms).await
}

#[cfg_attr(not(feature = "serial"), allow(dead_code))]
/// How likely a port name is the one to pick, lower first: a stable Linux
/// name under `/dev/serial/by-id`, a macOS call-out device (`/dev/cu.*`,
/// which opens without waiting for carrier, unlike `/dev/tty.*`) or a
/// Windows `COMn`; then the rest, and macOS dial-in devices last.
fn rank(name: &str) -> (u8, u32) {
    if name.starts_with("/dev/serial/by-id/") || name.starts_with("/dev/cu.") {
        (0, 0)
    } else if let Some(number) = name.strip_prefix("COM").and_then(|n| n.parse::<u32>().ok()) {
        (0, number)
    } else if name.starts_with("/dev/tty.") {
        (2, 0)
    } else {
        (1, 0)
    }
}

#[cfg_attr(not(feature = "serial"), allow(dead_code))]
/// `names` without repeats, preferred names first (see [`rank`]), and
/// otherwise in the order given.
pub(crate) fn order_ports(names: Vec<String>) -> Vec<String> {
    let mut names = dedup(names);
    names.sort_by_key(|name| rank(name));
    names
}

#[cfg_attr(not(feature = "serial"), allow(dead_code))]
/// `names` without repeats, each kept where it first appears.
pub(crate) fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

/// The serial ports the operating system reports, in [`order_ports`]'s
/// order. On Linux the stable `/dev/serial/by-id` names come first; the
/// kernel names they point to are still listed.
#[cfg(feature = "serial")]
pub(crate) fn list_ports() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/dev/serial/by-id") {
        let mut by_id: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_string_lossy().into_owned())
            .collect();
        by_id.sort();
        names.extend(by_id);
    }
    names.extend(bacnet_transport::mstp_serial::available_ports().map_err(|e| e.to_string())?);
    Ok(order_ports(names))
}

/// Without the `serial` feature there is nothing to list ports with.
#[cfg(not(feature = "serial"))]
pub(crate) fn list_ports() -> Result<Vec<String>, String> {
    Err(NO_SERIAL.into())
}

/// The advice for a build without the `serial` feature.
#[cfg_attr(all(feature = "serial", not(feature = "tui")), allow(dead_code))]
pub(crate) const NO_SERIAL: &str = "MS/TP requires the 'serial' feature. Rebuild with:\n  \
     cargo install bacnet-cli --features serial";

/// `bacnet mstp-ports`: the serial ports to pass to `--mstp`, as a JSON
/// array of names (possibly empty) or one name a line. Returns the exit
/// status; a failure leaves `out` empty.
pub(crate) fn ports_cmd(
    listed: Result<Vec<String>, String>,
    format: OutputFormat,
    out: &mut impl Write,
    err: &mut impl Write,
) -> u8 {
    let ports = match listed {
        Ok(ports) => ports,
        Err(error) => {
            let _ = writeln!(err, "Error: {error}");
            return 1;
        }
    };
    let written = match format {
        OutputFormat::Json => serde_json::to_writer_pretty(&mut *out, &ports)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(out)),
        OutputFormat::Table if ports.is_empty() => writeln!(out, "No serial ports found."),
        OutputFormat::Table => ports.iter().try_for_each(|port| writeln!(out, "{port}")),
    };
    u8::from(written.is_err())
}

/// Run a one-shot command over MS/TP.
#[cfg(feature = "serial")]
pub(crate) async fn run(
    cli: &crate::args::Cli,
    args: &crate::transport::TransportArgs,
    format: OutputFormat,
) -> crate::CliResult {
    let (client, _diagnostics) = build_client(args).await?;
    crate::run(client, cli, format).await
}

/// Without the `serial` feature, say how to get it.
#[cfg(not(feature = "serial"))]
pub(crate) async fn run(
    _cli: &crate::args::Cli,
    _args: &crate::transport::TransportArgs,
    _format: OutputFormat,
) -> crate::CliResult {
    eprintln!("Error: {NO_SERIAL}");
    std::process::exit(1);
}

#[cfg(test)]
#[path = "mstp_tests.rs"]
mod tests;
