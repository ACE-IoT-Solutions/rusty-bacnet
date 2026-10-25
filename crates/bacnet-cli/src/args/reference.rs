//! The website's CLI reference, generated from the clap definitions (#1989).
//!
//! `website/src/content/docs/reference/cli.md` holds authored notes around a
//! generated region between two marker comments. This test renders that
//! region from [`Cli`]'s clap `Command`: every subcommand with its usage,
//! arguments, options, defaults, value names and help text.
//!
//! It is a test rather than a hidden subcommand or a separate binary so that
//! the shipped `bacnet` carries none of it, and so that every CI build renders
//! the page (a panic here fails CI) without comparing it with the committed
//! copy: concurrent PRs that add an option would otherwise conflict over it.
//! `BACNET_CLI_REFERENCE` picks what else it does:
//!
//! - unset: render only, and check the page still has its markers;
//! - `write`: replace the page's generated region;
//! - `check`: fail if the page's generated region is stale.
//!
//! `website/scripts/reference-cli.mjs [--check]` runs it with every feature
//! on, so commands added behind a feature appear too; the release checklist
//! in `docs/ci.md` runs the check.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, CommandFactory};

use super::Cli;

/// Opens the generated region; the text up to the next `-->` is free.
const START: &str = "<!-- generated:start";
/// Closes the generated region.
const END: &str = "<!-- generated:end -->";

/// The Cargo feature a command or option needs; the clap tree has no record
/// of it, because the commands parse in every build and refuse to run in a
/// build without the feature. Names here must exist in the tree.
const COMMAND_FEATURES: &[(&str, &str)] = &[
    ("sc-check", "sc-tls"),
    ("tui", "tui"),
    ("capture", "pcap"),
    ("mstp-ports", "serial"),
];

/// Global or per-command long options that need a feature: (command or ""
/// for a global option, long option, feature).
const OPTION_FEATURES: &[(&str, &str, &str)] = &[
    ("", "sc", "sc-tls"),
    ("", "sc-url", "sc-tls"),
    ("", "sc-failover-url", "sc-tls"),
    ("", "sc-reconnect", "sc-tls"),
    ("", "sc-connect-timeout", "sc-tls"),
    ("", "sc-ca", "sc-tls"),
    ("", "sc-cert", "sc-tls"),
    ("", "sc-key", "sc-tls"),
    ("", "sc-vmac", "sc-tls"),
    ("", "sc-device-uuid", "sc-tls"),
    ("", "mstp", "serial"),
    ("", "mstp-baud", "serial"),
    ("", "mstp-mac", "serial"),
    ("", "mstp-max-master", "serial"),
    ("", "mstp-max-info-frames", "serial"),
    ("", "mstp-rt-priority", "serial"),
    ("", "mstp-cpus", "serial"),
    ("tui", "mstp-monitor", "serial"),
    ("tui", "capture-device", "pcap"),
];

fn page_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/src/content/docs/reference/cli.md")
}

/// Replace the text between the markers of `page` with `region`.
fn splice(page: &str, region: &str) -> Result<String, String> {
    let start = page
        .find(START)
        .ok_or("the page has no generated:start marker")?;
    let open = start
        + page[start..]
            .find("-->")
            .ok_or("the generated:start marker is not closed")?
        + 3;
    let end = page[open..]
        .find(END)
        .map(|at| open + at)
        .ok_or("the page has no generated:end marker after generated:start")?;
    Ok(format!("{}\n\n{}\n{}", &page[..open], region, &page[end..]))
}

/// Escape text that clap prints for Markdown, leaving code spans alone.
fn markdown_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, part) in text.split('`').enumerate() {
        if i > 0 {
            out.push('`');
        }
        if i % 2 == 1 {
            out.push_str(part);
            continue;
        }
        for c in part.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '*' | '_' | '[' | ']' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
    }
    out
}

/// One paragraph per blank-line-separated block of help text, each ending in
/// a full stop (clap drops the one that ends a one-paragraph doc comment).
fn paragraphs(text: &str) -> String {
    text.split("\n\n")
        .map(|block| block.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|block| !block.is_empty())
        .map(|mut block| {
            if !block.ends_with(['.', '!', '?', ':']) {
                block.push('.');
            }
            markdown_text(&block)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Whether help text already says it needs `feature`.
fn mentions(help: &str, feature: &str) -> bool {
    help.contains(&format!("{feature} feature")) || help.contains(&format!("`{feature}` feature"))
}

fn help_of(arg: &Arg) -> String {
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .map(|help| help.to_string())
        .unwrap_or_default()
}

/// The `<NAME>` placeholders an argument takes, or "" for a flag.
fn placeholder(arg: &Arg) -> String {
    if !arg.get_action().takes_values() {
        return String::new();
    }
    let names: Vec<String> = match arg.get_value_names() {
        Some(names) => names.iter().map(ToString::to_string).collect(),
        None => vec![arg.get_id().as_str().to_uppercase()],
    };
    let many = arg
        .get_num_args()
        .is_some_and(|range| range.max_values() > 1);
    names
        .iter()
        .map(|name| format!("<{name}>{}", if many { "..." } else { "" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How an option is written: `-i, --interface <INTERFACE>`.
fn option_syntax(arg: &Arg) -> String {
    let mut forms = Vec::new();
    if let Some(short) = arg.get_short() {
        forms.push(format!("-{short}"));
    }
    if let Some(long) = arg.get_long() {
        forms.push(format!("--{long}"));
    }
    let value = placeholder(arg);
    if value.is_empty() {
        forms.join(", ")
    } else {
        format!("{} {value}", forms.join(", "))
    }
}

/// The facts after an argument's help: possible values, default, feature.
fn facts(arg: &Arg, feature: Option<&str>) -> String {
    let mut facts = Vec::new();
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| format!("`{}`", value.get_name()))
        .collect();
    if !values.is_empty() && arg.get_action().takes_values() {
        facts.push(format!("One of {}.", values.join(", ")));
    }
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|value| format!("`{}`", value.to_string_lossy()))
        .collect();
    if !defaults.is_empty() && arg.get_action().takes_values() {
        facts.push(format!("Default: {}.", defaults.join(", ")));
    }
    if arg.is_required_set() {
        facts.push("Required.".into());
    }
    if matches!(arg.get_action(), ArgAction::Count) {
        facts.push("Repeat it for more.".into());
    }
    if let Some(feature) = feature {
        facts.push(format!("Needs the `{feature}` feature."));
    }
    facts.join(" ")
}

fn option_feature(command: &str, long: Option<&str>) -> Option<&'static str> {
    let long = long?;
    OPTION_FEATURES
        .iter()
        .find(|(owner, name, _)| *owner == command && *name == long)
        .map(|(_, _, feature)| *feature)
}

fn command_feature(command: &str) -> Option<&'static str> {
    COMMAND_FEATURES
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, feature)| *feature)
}

/// One bullet per argument.
fn argument_list(out: &mut String, command: &str, args: &[&Arg]) {
    for arg in args {
        let syntax = if arg.is_positional() {
            placeholder(arg)
        } else {
            option_syntax(arg)
        };
        let raw = help_of(arg);
        let help = paragraphs(&raw).replace("\n\n", " ");
        let feature =
            option_feature(command, arg.get_long()).filter(|feature| !mentions(&raw, feature));
        let facts = facts(arg, feature);
        let text = [help, facts]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(out, "- `{syntax}`: {text}").expect("write to a String");
    }
}

/// An argument the page lists under a command: not hidden, not a global
/// option clap copied down from `bacnet`, and not the generated help flag.
fn listed(arg: &Arg) -> bool {
    !arg.is_hide_set()
        && !arg.is_global_set()
        && arg.get_id() != "help"
        && arg.get_id() != "version"
}

/// The `bacnet` options every command takes.
fn global_options(cli: &Command) -> Vec<&Arg> {
    cli.get_arguments()
        .filter(|arg| !arg.is_hide_set() && !arg.is_positional())
        .collect()
}

/// The generated region of the page.
fn render() -> String {
    let mut cli = Cli::command();
    cli.build();
    let mut out = String::new();
    let w = &mut out;

    writeln!(w, "## Usage\n").expect("write to a String");
    writeln!(
        w,
        "```text\n{}\n```\n",
        cli.render_usage().to_string().trim()
    )
    .expect("write to a String");
    if let Some(about) = cli.get_about() {
        writeln!(w, "{}\n", paragraphs(&about.to_string())).expect("write to a String");
    }

    writeln!(w, "## Global options\n").expect("write to a String");
    writeln!(
        w,
        "These options go before or after the command. Each command also takes `-h` or `--help`.\n"
    )
    .expect("write to a String");
    argument_list(w, "", &global_options(&cli));
    writeln!(w).expect("write to a String");

    writeln!(w, "## Commands\n").expect("write to a String");
    writeln!(w, "| Command | Aliases | Summary |\n|---|---|---|").expect("write to a String");
    let commands: Vec<&Command> = cli
        .get_subcommands()
        .filter(|command| !command.is_hide_set() && command.get_name() != "help")
        .collect();
    for command in &commands {
        let aliases: Vec<String> = command
            .get_all_aliases()
            .map(|alias| format!("`{alias}`"))
            .collect();
        let mut summary = command
            .get_about()
            .map(|about| paragraphs(&about.to_string()).replace("\n\n", " "))
            .unwrap_or_default();
        if let Some(feature) = command_feature(command.get_name()) {
            summary.push_str(&format!(" Needs the `{feature}` feature."));
        }
        writeln!(
            w,
            "| [`{name}`](#bacnet-{name}) | {} | {} |",
            if aliases.is_empty() {
                "—".into()
            } else {
                aliases.join(", ")
            },
            summary.replace('|', "\\|"),
            name = command.get_name(),
        )
        .expect("write to a String");
    }
    writeln!(w).expect("write to a String");

    for command in &commands {
        let name = command.get_name();
        writeln!(w, "### bacnet {name}\n").expect("write to a String");
        let about = command
            .get_long_about()
            .or_else(|| command.get_about())
            .map(|about| about.to_string())
            .unwrap_or_default();
        if !about.is_empty() {
            writeln!(w, "{}\n", paragraphs(&about)).expect("write to a String");
        }
        if let Some(feature) = command_feature(name) {
            writeln!(w, "Needs the `{feature}` feature.\n").expect("write to a String");
        }
        let aliases: Vec<String> = command
            .get_all_aliases()
            .map(|alias| format!("`{alias}`"))
            .collect();
        if !aliases.is_empty() {
            writeln!(w, "Aliases: {}.\n", aliases.join(", ")).expect("write to a String");
        }
        let usage = (*command).clone().render_usage().to_string();
        writeln!(w, "```text\n{}\n```\n", usage.trim()).expect("write to a String");
        let args: Vec<&Arg> = command.get_arguments().filter(|arg| listed(arg)).collect();
        let positionals: Vec<&Arg> = args
            .iter()
            .copied()
            .filter(|arg| arg.is_positional())
            .collect();
        let options: Vec<&Arg> = args
            .iter()
            .copied()
            .filter(|arg| !arg.is_positional())
            .collect();
        if !positionals.is_empty() {
            writeln!(w, "Arguments:\n").expect("write to a String");
            argument_list(w, name, &positionals);
            writeln!(w).expect("write to a String");
        }
        if !options.is_empty() {
            writeln!(w, "Options:\n").expect("write to a String");
            argument_list(w, name, &options);
            writeln!(w).expect("write to a String");
        }
    }
    out.trim_end().to_string()
}

/// Every feature-table entry names a command or option in the tree, so a
/// rename can't leave a stale mark behind.
fn check_feature_tables(cli: &Command) {
    for (name, _) in COMMAND_FEATURES {
        assert!(
            cli.find_subcommand(name).is_some(),
            "COMMAND_FEATURES names `{name}`, which is not a bacnet command"
        );
    }
    for (owner, long, _) in OPTION_FEATURES {
        let command = if owner.is_empty() {
            cli
        } else {
            cli.find_subcommand(owner)
                .unwrap_or_else(|| panic!("OPTION_FEATURES names command `{owner}`"))
        };
        assert!(
            command
                .get_arguments()
                .any(|arg| arg.get_long() == Some(long)),
            "OPTION_FEATURES names `--{long}`, which `{}` does not take",
            command.get_name()
        );
    }
}

#[test]
fn cli_reference() {
    // Building the clap tree takes the parse thread's stack in debug builds.
    let region = crate::parse_on_large_stack(|| {
        check_feature_tables(&Cli::command());
        render()
    });
    for command in Cli::command().get_subcommands() {
        assert!(
            region.contains(&format!("### bacnet {}\n", command.get_name())),
            "{} has a section",
            command.get_name()
        );
    }
    let path = page_path();
    let page = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let fresh =
        splice(&page, &region).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    match std::env::var("BACNET_CLI_REFERENCE").as_deref() {
        Ok("write") => {
            if fresh != page {
                std::fs::write(&path, &fresh)
                    .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
            }
        }
        Ok("check") => assert!(
            fresh == page,
            "{} is stale; regenerate it with `node website/scripts/reference-cli.mjs`",
            path.display()
        ),
        Ok(other) => panic!("BACNET_CLI_REFERENCE is `write` or `check`, not `{other}`"),
        Err(_) => {}
    }
}

#[test]
fn splice_replaces_only_the_region() {
    let page = "intro\n<!-- generated:start by a script -->\nold\n<!-- generated:end -->\noutro\n";
    assert_eq!(
        splice(page, "new").unwrap(),
        "intro\n<!-- generated:start by a script -->\n\nnew\n<!-- generated:end -->\noutro\n"
    );
    assert!(splice("no markers", "new").is_err());
    assert!(splice("<!-- generated:end --> <!-- generated:start -->", "new").is_err());
}

#[test]
fn markdown_text_escapes_outside_code_spans() {
    assert_eq!(
        markdown_text("a <b> *c* `<d> *e*` f_g"),
        "a &lt;b&gt; \\*c\\* `<d> *e*` f\\_g"
    );
}
