//! The MS/TP flags, the port list and the client the CLI builds (#969).

use std::time::Duration;

use bacnet_server::server::BACnetServer;
use bacnet_transport::mstp::{LoopbackSerial, MstpTransport};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::*;
use crate::core::destination::{self, Destination};

/// A device with one object, AI:2 at 51, and the APDU limit of a
/// standard-frame MS/TP port.
fn server_database() -> (
    bacnet_objects::DeviceIdentity,
    bacnet_objects::database::ObjectDatabase,
) {
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::traits::BACnetObject;
    let identity = bacnet_objects::DeviceIdentity::new(SERVER_INSTANCE, 260)
        .and_then(|identity| identity.with_max_apdu(480))
        .unwrap();
    let mut device = DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..Default::default()
    })
    .unwrap();
    let mut ai2 = AnalogInputObject::new(
        2,
        "Outside Air",
        bacnet_types::enums::EngineeringUnits::DEGREES_FAHRENHEIT,
    )
    .unwrap();
    ai2.set_present_value(51.0);
    device.set_object_list(vec![device.object_identifier(), ai2.object_identifier()]);
    let mut db = bacnet_objects::database::ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(ai2)).unwrap();
    (identity, db)
}

/// Parse a command line on a large stack, as `main` does (#953).
fn parse<const N: usize>(args: [&'static str; N]) -> Result<crate::args::Cli, clap::Error> {
    crate::parse_on_large_stack(move || {
        use clap::{CommandFactory, FromArgMatches};
        let mut matches = crate::args::Cli::command().try_get_matches_from(args)?;
        crate::args::capture_sources(&matches)?;
        crate::args::Cli::from_arg_matches_mut(&mut matches)
    })
}

/// [`parse`] for a command line built at run time.
fn parse_vec(args: Vec<&'static str>) -> Result<crate::args::Cli, clap::Error> {
    crate::parse_on_large_stack(move || {
        use clap::{CommandFactory, FromArgMatches};
        let mut matches = crate::args::Cli::command().try_get_matches_from(args)?;
        crate::args::capture_sources(&matches)?;
        crate::args::Cli::from_arg_matches_mut(&mut matches)
    })
}

#[test]
fn each_flag_names_its_allowed_values() {
    for rate in BAUD_RATES {
        assert_eq!(parse_baud(&rate.to_string()), Ok(rate));
    }
    for bad in ["0", "12345", "4800", "230400", "fast"] {
        let error = parse_baud(bad).unwrap_err();
        assert!(error.contains("9600, 19200, 38400, 57600, 76800 or 115200"));
    }
    assert_eq!(parse_master_mac("0"), Ok(0));
    assert_eq!(parse_master_mac("127"), Ok(127));
    for bad in ["128", "254", "-1", "x"] {
        assert!(
            parse_master_mac(bad).unwrap_err().contains("0 to 127"),
            "{bad}"
        );
    }
    assert_eq!(parse_max_master("127"), Ok(127));
    assert!(parse_max_master("128").unwrap_err().contains("0 to 127"));
    assert_eq!(parse_max_info_frames("1"), Ok(1));
    assert_eq!(parse_max_info_frames("255"), Ok(255));
    for bad in ["0", "256"] {
        assert!(parse_max_info_frames(bad).unwrap_err().contains("1 to 255"));
    }
}

/// `tui --mstp-monitor PORT` takes the baud rate and Max_Info_Frames from
/// either side of the subcommand, and refuses `--mstp`, the flags the join
/// wizard proposes, other transports' flags and a profile (#970).
#[test]
fn the_monitor_flag_takes_baud_and_refuses_what_it_proposes() {
    let cli = parse([
        "bacnet",
        "--mstp-baud",
        "19200",
        "tui",
        "--mstp-monitor",
        "/dev/ttyUSB1",
        "--mstp-max-info-frames",
        "3",
    ])
    .unwrap();
    let Some(crate::args::Command::Tui { mstp_monitor, .. }) = &cli.command else {
        panic!("expected tui");
    };
    assert_eq!(mstp_monitor.as_deref(), Some("/dev/ttyUSB1"));
    assert_eq!((cli.mstp_baud, cli.mstp_max_info_frames), (19_200, 3));
    for (args, diagnostic) in [
        (
            vec![
                "bacnet",
                "--mstp",
                "/dev/x",
                "tui",
                "--mstp-monitor",
                "/dev/y",
            ],
            "--mstp cannot be used with --mstp-monitor",
        ),
        (
            vec![
                "bacnet",
                "tui",
                "--mstp-monitor",
                "/dev/y",
                "--mstp-mac",
                "3",
            ],
            "--mstp-mac cannot be used with --mstp-monitor: the join wizard (J) proposes it",
        ),
        (
            vec![
                "bacnet",
                "--mstp-max-master",
                "9",
                "tui",
                "--mstp-monitor",
                "/dev/y",
            ],
            "--mstp-max-master cannot be used with --mstp-monitor",
        ),
        (
            vec!["bacnet", "tui", "--mstp-monitor", "/dev/y", "--sc"],
            "--sc cannot be used with --mstp-monitor: it belongs to another transport",
        ),
        (
            vec!["bacnet", "tui", "--mstp-monitor", " "],
            "--mstp-monitor needs a serial port name",
        ),
        (
            vec![
                "bacnet",
                "tui",
                "--mstp-monitor",
                "/dev/y",
                "--profile",
                "lab",
            ],
            "cannot be used with",
        ),
        (
            vec!["bacnet", "devices", "--mstp-baud", "9600"],
            "--mstp-baud requires --mstp PORT (or tui --mstp-monitor PORT)",
        ),
    ] {
        let error = parse_vec(args.clone()).unwrap_err().to_string();
        assert!(error.contains(diagnostic), "{args:?}: {error}");
    }
}

#[test]
fn the_flags_reach_the_transport_settings() {
    let cli = parse([
        "bacnet",
        "--mstp",
        "/dev/ttyUSB0",
        "--mstp-mac",
        "3",
        "devices",
        "--mstp-baud",
        "76800",
        "--mstp-max-master",
        "10",
        "--mstp-max-info-frames",
        "4",
    ])
    .unwrap();
    let args = crate::transport::TransportArgs::from_cli(
        &cli,
        std::net::Ipv4Addr::UNSPECIFIED,
        std::net::Ipv4Addr::BROADCAST,
    )
    .unwrap();
    let mstp = args.mstp.unwrap();
    assert_eq!(
        mstp,
        MstpArgs {
            port: "/dev/ttyUSB0".into(),
            baud: 76_800,
            mac: 3,
            max_master: 10,
            max_info_frames: 4,
        }
    );
    let config = mstp.config();
    assert_eq!(
        (
            config.this_station,
            config.max_master,
            config.max_info_frames
        ),
        (3, 10, 4)
    );
    assert_eq!(config.baud_rate, 76_800);
    // The defaults, with only the port and MAC given.
    let cli = parse(["bacnet", "--mstp", "COM3", "--mstp-mac", "0", "devices"]).unwrap();
    assert_eq!(
        (cli.mstp_baud, cli.mstp_max_master, cli.mstp_max_info_frames),
        (38_400, 127, 1)
    );
}

/// Two-flag checks fail at parse time, as usage errors.
#[test]
fn the_cross_flag_checks_are_usage_errors() {
    use clap::error::ErrorKind;
    let error = parse(["bacnet", "--mstp", "/dev/x", "devices"]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    assert!(
        error.to_string().contains("--mstp-mac (0 to 127)"),
        "{error}"
    );
    let error = parse([
        "bacnet",
        "--mstp",
        "/dev/x",
        "--mstp-mac",
        "9",
        "--mstp-max-master",
        "5",
        "devices",
    ])
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
    assert!(
        error.to_string().contains("above --mstp-max-master 5"),
        "{error}"
    );
    let error = parse(["bacnet", "--mstp-mac", "9", "devices"]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    // Another transport's flag, before or after the subcommand, on the
    // same side as --mstp or the other one: refused, never dropped.
    let mstp = ["--mstp", "/dev/x", "--mstp-mac", "1"];
    for other in [
        &["--sc"][..],
        &["--ipv6"],
        &["--foreign-bbmd", "192.0.2.10"],
        &["-i", "10.0.0.5"],
        &["--sc-url", "wss://hub:443"],
        &["--ttl", "60"],
        &["-p", "47809"],
        &["--device-instance", "5"],
    ] {
        let shapes = [
            [&["bacnet"][..], &mstp, other, &["devices"]].concat(),
            [&["bacnet"][..], other, &["devices"], &mstp].concat(),
            [&["bacnet"][..], &mstp, &["devices"], other].concat(),
            [&["bacnet"][..], &["devices"], other, &mstp].concat(),
        ];
        for line in shapes {
            let error = parse_vec(line.clone()).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict, "{line:?}");
            assert!(
                error.to_string().contains("cannot be used with --mstp"),
                "{error}"
            );
        }
    }
    for empty in ["", " "] {
        let error = parse_vec(vec![
            "bacnet",
            "--mstp",
            empty,
            "--mstp-mac",
            "1",
            "devices",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert!(
            error.to_string().contains("needs a serial port name"),
            "{error}"
        );
    }
    assert!(parse(["bacnet", "devices"]).unwrap().mstp.is_none());
    // The other transports' flags are still fine without --mstp.
    assert!(parse_vec(vec!["bacnet", "--sc", "devices", "--ttl", "60"]).is_ok());
}

/// A profile's settings get the flags' checks, naming its keys.
#[test]
fn settings_from_elsewhere_are_checked_too() {
    let good = MstpArgs {
        port: "/dev/ttyUSB0".into(),
        baud: 38_400,
        mac: 3,
        max_master: 127,
        max_info_frames: 1,
    };
    assert_eq!(good.validate(), Ok(()));
    for (bad, expected) in [
        (
            MstpArgs {
                port: " ".into(),
                ..good.clone()
            },
            "serial port is required",
        ),
        (
            MstpArgs {
                baud: 1200,
                ..good.clone()
            },
            "baud must be one of",
        ),
        (
            MstpArgs {
                mac: 128,
                max_master: 127,
                ..good.clone()
            },
            "mac a master station's MAC must be 0 to 127",
        ),
        (
            MstpArgs {
                mac: 9,
                max_master: 5,
                ..good.clone()
            },
            "mac 9 is above max_master 5",
        ),
        (
            MstpArgs {
                max_info_frames: 0,
                ..good.clone()
            },
            "max_info_frames must be 1 to 255",
        ),
    ] {
        let error = bad.validate().unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn ports_are_listed_preferred_first_without_repeats() {
    let ordered = order_ports(
        [
            "/dev/tty.usbserial-1",
            "/dev/ttyUSB0",
            "COM10",
            "/dev/cu.usbserial-1",
            "/dev/serial/by-id/usb-FTDI-if00",
            "COM3",
            "/dev/ttyUSB0",
        ]
        .map(String::from)
        .to_vec(),
    );
    assert_eq!(
        ordered,
        [
            "/dev/cu.usbserial-1",
            "/dev/serial/by-id/usb-FTDI-if00",
            "COM3",
            "COM10",
            "/dev/ttyUSB0",
            "/dev/tty.usbserial-1",
        ]
    );
}

#[test]
fn mstp_ports_prints_an_array_or_a_line_each_and_fails_on_stderr_only() {
    let run = |listed: Result<Vec<String>, String>, format| {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = ports_cmd(listed, format, &mut out, &mut err);
        (
            status,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    };
    let (status, out, _) = run(Ok(vec![]), OutputFormat::Json);
    assert_eq!((status, out.as_str()), (0, "[]\n"));
    let ports = vec!["/dev/cu.a".to_string(), "COM3".to_string()];
    let (status, out, _) = run(Ok(ports.clone()), OutputFormat::Json);
    assert_eq!(status, 0);
    assert_eq!(serde_json::from_str::<Vec<String>>(&out).unwrap(), ports);
    let (_, out, _) = run(Ok(ports), OutputFormat::Table);
    assert_eq!(out, "/dev/cu.a\nCOM3\n");
    let (_, out, _) = run(Ok(vec![]), OutputFormat::Table);
    assert_eq!(out, "No serial ports found.\n");
    let (status, out, err) = run(Err("no listing".into()), OutputFormat::Json);
    assert_eq!((status, out.as_str()), (1, ""));
    assert_eq!(err, "Error: no listing\n");
}

/// The listing works wherever the feature builds; there may be no port.
#[cfg(feature = "serial")]
#[test]
fn the_system_lists_its_ports() {
    match list_ports() {
        Ok(ports) => assert_eq!(order_ports(ports.clone()), ports),
        // A container without sysfs can't be asked.
        Err(error) => assert!(error.contains("listing failed"), "{error}"),
    }
}

const SERVER_MAC: u8 = 1;
const CLIENT_MAC: u8 = 3;
const SERVER_INSTANCE: u32 = 4001;

/// The acceptance check: a client built with the CLI's MS/TP setup, at MAC
/// 3, finds a stack server at MAC 1 on a loopback line and reads from it,
/// by instance and by `mstp:1`, and the address reads back as `mstp:1`.
#[tokio::test]
async fn the_cli_client_discovers_and_reads_a_server_on_the_trunk() {
    let (server_line, client_line) = LoopbackSerial::pair();
    let settings = |mac| MstpArgs {
        port: "loopback".into(),
        baud: 38_400,
        mac,
        max_master: 3,
        max_info_frames: 1,
    };
    let (identity, database) = server_database();
    let mut server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(MstpTransport::new(
            server_line,
            settings(SERVER_MAC).config(),
        ))
        .build()
        .await
        .unwrap();
    let mode = bacnet_transport::mstp::MstpExecutionMode::DedicatedThread;
    let (mut client, diagnostics) =
        build_client_on(client_line, &settings(CLIENT_MAC), 6000, 0, mode)
            .await
            .unwrap();
    assert_eq!(client.local_mac(), [CLIENT_MAC]);
    let style = crate::core::target::AddressStyle::of(client.transport());
    assert_eq!(style, crate::core::target::AddressStyle::Mstp);

    let deadline = Duration::from_secs(30);
    let device = tokio::time::timeout(deadline, async {
        loop {
            client.who_is(None).await.unwrap();
            for _ in 0..20 {
                if let Some(device) = client.get_device(SERVER_INSTANCE).await {
                    return device;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    })
    .await
    .expect("the server answered the Who-Is");
    assert_eq!(device.mac_address.as_slice(), [SERVER_MAC]);
    assert_eq!(style.format(&device.mac_address), "mstp:1");

    let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    for target in ["4001", "mstp:1", "mstp:0x01"] {
        let to = destination::resolve(&client, target, Duration::from_secs(1))
            .await
            .unwrap();
        let ack = match &to {
            Destination::Mac(mac) => {
                assert_eq!(mac.as_slice(), [SERVER_MAC], "{target}");
                client
                    .read_property(mac, ai2, PropertyIdentifier::PRESENT_VALUE, None)
                    .await
            }
            Destination::Device(instance) => {
                client
                    .read_property_from_device(
                        *instance,
                        ai2,
                        PropertyIdentifier::PRESENT_VALUE,
                        None,
                    )
                    .await
            }
        }
        .unwrap();
        let (value, _) =
            bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
        assert_eq!(value, PropertyValue::Real(51.0), "{target}");
    }
    // A target in another transport's form is refused before sending.
    for other in [
        "10.0.0.1",
        "02:00:00:00:00:01",
        "bip6:5",
        "mstp:255",
        "mstp:x",
    ] {
        assert!(
            destination::resolve(&client, other, Duration::from_secs(1))
                .await
                .is_err(),
            "{other}"
        );
    }

    // The node's state: in the ring with the server, not alone.
    let node = diagnostics.node_state().expect("published");
    assert!(!node.sole_master);
    assert!(diagnostics.snapshot().mac.token_rotations > 0);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
