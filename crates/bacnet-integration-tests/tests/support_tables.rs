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

const STUB: &str = "x\nclass BACnetClient:\n    def __init__(self) -> None: ...\n    def read_property(\n    async def who_has_by_id(\n    def vt_open(\n    def subscribe_cov_property_multiple(\n    def read_property_from_devices(\nclass Other:\n    def write_property(\n";

#[test]
fn python_methods_match_services_by_name() {
    use bacnet_types::enums::ServiceSupported as S;
    let found = support_tables::python_services_with(STUB, &["__init__"]).unwrap();
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

#[test]
fn every_python_method_is_classified() {
    // A method that names no service and isn't listed fails generation.
    let stub = STUB.replace("class Other:", "    def who_is_listening(\nclass Other:");
    let error = support_tables::python_services_with(&stub, &["__init__"]).unwrap_err();
    assert!(error.contains("who_is_listening"), "{error}");
    // So does a listed method the stub no longer has.
    let error = support_tables::python_services_with(STUB, &["__init__", "gone"]).unwrap_err();
    assert!(error.contains("gone"), "{error}");
    // A listed method counts for no service, even when its name would match one.
    let stub = STUB.replace("class Other:", "    def who_is_replies(\nclass Other:");
    let found =
        support_tables::python_services_with(&stub, &["__init__", "who_is_replies"]).unwrap();
    let who_is = bacnet_types::enums::ServiceSupported::WHO_IS.to_raw();
    assert!(!found.contains(&who_is));
    // The real stub is fully classified.
    let real =
        std::fs::read_to_string(repo().join("crates/rusty-bacnet/rusty_bacnet.pyi")).unwrap();
    support_tables::python_services(&real).unwrap();
}

#[test]
fn the_feature_resolver_refuses_what_it_cannot_resolve() {
    let error = support_tables::transport_features(&repo(), "bacnet-cli", &["no-such-feature"])
        .unwrap_err();
    assert!(error.contains("no feature no-such-feature"), "{error}");
    let linux =
        support_tables::transport_features(&repo(), "bacnet-cli", &["sc-tls", "serial"]).unwrap();
    assert!(
        linux.contains("serial") && linux.contains("sc-tls") && linux.contains("ipv6"),
        "{linux:?}"
    );
    let default = support_tables::transport_features(&repo(), "bacnet-cli", &[]).unwrap();
    assert!(!default.contains("serial"), "{default:?}");
}
