//! The committed capability tables match a fresh run of their generator (#1496).
#![forbid(unsafe_code)]

#[path = "../support_tables.rs"]
mod support_tables;

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_committed_support_tables_are_fresh() {
    let fresh = support_tables::generate(&repo()).unwrap();
    let committed = std::fs::read_to_string(repo().join(support_tables::OUTPUT))
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == fresh,
        "{} is stale: run `cargo run -p bacnet-integration-tests --example support-tables` and commit the result",
        support_tables::OUTPUT
    );
}

#[test]
fn the_tables_never_claim_conformance() {
    let committed = std::fs::read_to_string(repo().join(support_tables::OUTPUT))
        .unwrap()
        .to_lowercase();
    for word in support_tables::FORBIDDEN_WORDS {
        assert!(!committed.contains(word), "support.json says {word:?}");
    }
}

#[test]
fn python_methods_match_services_by_name() {
    use bacnet_types::enums::ServiceSupported as S;
    let stub = "x\nclass BACnetClient:\n    def __init__(self) -> None: ...\n    def read_property(\n    async def who_has_by_id(\n    def vt_open(\n    def subscribe_cov_property_multiple(\n    def read_property_from_devices(\nclass Other:\n    def write_property(\n";
    let found = support_tables::python_services(stub).unwrap();
    let expected: std::collections::BTreeSet<u8> = [
        S::READ_PROPERTY,
        S::WHO_HAS,
        S::VT_OPEN,
        S::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ]
    .iter()
    .map(|s| s.to_raw())
    .collect();
    // Not SubscribeCOVProperty (a prefix of a longer service), and nothing
    // from another class.
    assert_eq!(found, expected);
}
