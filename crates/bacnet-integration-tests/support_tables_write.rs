//! Rewrites `website/src/data/support.json` from the code (#1496):
//! `cargo run -p bacnet-integration-tests --example support-tables`.
#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool reports what it did

#[path = "support_tables.rs"]
mod support_tables;

fn main() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = support_tables::generate(&repo).unwrap_or_else(|error| {
        eprintln!("support-tables: {error}");
        std::process::exit(1);
    });
    let path = repo.join(support_tables::OUTPUT);
    std::fs::write(&path, text).unwrap_or_else(|error| {
        eprintln!("support-tables: write {}: {error}", path.display());
        std::process::exit(1);
    });
    println!("wrote {}", support_tables::OUTPUT);
}
