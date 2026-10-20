//! `--mstp-rt-priority` and `--mstp-cpus` (#501): how the thread that runs
//! this station's token handling is scheduled. They are settings of the
//! host, not of a trunk, so they come from the command line for every
//! station the process joins as (`--mstp`, a profile, or a join from the
//! monitor) and are not saved in profiles.
//!
//! Both are Linux only, and refused as they are parsed elsewhere. A setting
//! the kernel won't apply fails the join with the reason.

use bacnet_transport::mstp::{MstpExecutionMode, MstpThreadConfig};

/// The flags' clap ids.
pub(crate) const FLAGS: [&str; 2] = ["mstp_rt_priority", "mstp_cpus"];

/// `--mstp-rt-priority`: a `SCHED_FIFO` priority, 1 to 99, on Linux.
pub(crate) fn parse_rt_priority(text: &str) -> Result<u8, String> {
    let priority = text
        .parse::<u8>()
        .map_err(|_| "must be 1 to 99".to_string())?;
    MstpThreadConfig::new()
        .with_realtime_priority(priority)
        .validate()
        .map_err(|error| without_prefix(&error))?;
    Ok(priority)
}

/// One CPU of `--mstp-cpus`, numbered from 0 as Linux numbers them.
pub(crate) fn parse_cpu(text: &str) -> Result<usize, String> {
    let cpu = text
        .trim()
        .parse::<usize>()
        .map_err(|_| "must be CPU numbers, such as 2 or 2,3".to_string())?;
    MstpThreadConfig::new()
        .with_cpu_affinity([cpu])
        .validate()
        .map_err(|error| without_prefix(&error))?;
    Ok(cpu)
}

/// The transport's message, without the error kind's prefix.
fn without_prefix(error: &bacnet_types::error::Error) -> String {
    match error {
        bacnet_types::error::Error::Encoding(message) => message.clone(),
        other => other.to_string(),
    }
}

/// The flags' settings.
#[cfg_attr(not(any(feature = "serial", feature = "tui")), allow(dead_code))]
pub(crate) fn from_cli(cli: &crate::args::Cli) -> MstpThreadConfig {
    MstpThreadConfig {
        realtime_priority: cli.mstp_rt_priority,
        cpu_affinity: cli.mstp_cpus.clone(),
    }
}

/// The CLI runs a station's loop on a thread of its own, so a busy
/// application runtime doesn't delay token handling; `thread` says how
/// that thread is scheduled.
#[cfg_attr(not(any(feature = "serial", feature = "tui")), allow(dead_code))]
pub(crate) fn mode(thread: &MstpThreadConfig) -> MstpExecutionMode {
    MstpExecutionMode::DedicatedThread(thread.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a command line on a large stack, as `main` does (#953).
    fn parse(args: Vec<&'static str>) -> Result<crate::args::Cli, clap::Error> {
        crate::parse_on_large_stack(move || {
            use clap::{CommandFactory, FromArgMatches};
            let mut matches = crate::args::Cli::command().try_get_matches_from(args)?;
            crate::args::capture_sources(&matches)?;
            crate::args::Cli::from_arg_matches_mut(&mut matches)
        })
    }

    /// A one-shot command takes the flags only with `--mstp`; the terminal
    /// UI takes them alone, for a station it joins later.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_flags_need_a_station_to_schedule() {
        let error = parse(vec!["bacnet", "discover", "--mstp-cpus", "0"]).unwrap_err();
        let error = error.to_string();
        assert!(
            error.contains("--mstp-cpus requires --mstp PORT"),
            "{error}"
        );
        for args in [
            vec![
                "bacnet",
                "tui",
                "--mstp-rt-priority",
                "50",
                "--mstp-cpus",
                "1,0",
            ],
            vec!["bacnet", "--mstp-rt-priority", "50", "--mstp-cpus", "1,0"],
            vec![
                "bacnet",
                "--mstp",
                "/dev/ttyUSB0",
                "--mstp-mac",
                "3",
                "discover",
                "--mstp-rt-priority",
                "50",
                "--mstp-cpus",
                "1,0",
            ],
        ] {
            let cli = parse(args.clone()).unwrap();
            let thread = from_cli(&cli);
            assert_eq!(thread.realtime_priority, Some(50), "{args:?}");
            assert_eq!(thread.cpu_affinity, [1, 0], "{args:?}");
            assert_eq!(mode(&thread), MstpExecutionMode::DedicatedThread(thread));
        }
        let cli = parse(vec!["bacnet", "tui"]).unwrap();
        assert_eq!(from_cli(&cli), MstpThreadConfig::new());
    }

    /// Elsewhere the flags fail as they are parsed, with the reason.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_flags_are_refused_off_linux() {
        for flag in ["--mstp-rt-priority", "--mstp-cpus"] {
            let error = parse(vec!["bacnet", "tui", flag, "1"]).unwrap_err();
            let error = error.to_string();
            assert!(error.contains("Linux only"), "{flag}: {error}");
        }
    }

    #[test]
    fn priorities_outside_1_to_99_are_refused() {
        for bad in ["0", "100", "-1", "x"] {
            assert!(parse_rt_priority(bad).is_err(), "{bad}");
        }
        let error = parse_cpu("1024").unwrap_err();
        assert!(error.contains("CPU 1024"), "{error}");
        assert!(parse_cpu("two").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_takes_both() {
        assert_eq!(parse_rt_priority("50"), Ok(50));
        assert_eq!(parse_cpu(" 3"), Ok(3));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn elsewhere_both_are_refused() {
        for error in [
            parse_rt_priority("50").unwrap_err(),
            parse_cpu("3").unwrap_err(),
        ] {
            assert!(error.contains("Linux only"), "{error}");
        }
    }
}
