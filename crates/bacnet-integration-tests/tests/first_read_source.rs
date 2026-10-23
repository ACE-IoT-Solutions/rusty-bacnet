//! The guide is the source of both the executable and its dependency contract.
#![forbid(unsafe_code)]

#[path = "../sample_source.rs"]
mod sample_source;

const GUIDE: &[u8] = include_bytes!("../../../website/src/content/docs/start/rust.md");

#[test]
fn generated_files_are_the_exact_guide_fences() {
    assert_eq!(
        sample_source::extract(GUIDE, "rust").unwrap(),
        include_bytes!(concat!(env!("OUT_DIR"), "/first_read.rs"))
    );
    assert_eq!(
        sample_source::extract(GUIDE, "toml").unwrap(),
        include_bytes!(concat!(env!("OUT_DIR"), "/first_read.toml"))
    );
}

#[test]
fn extraction_preserves_crlf_blank_lines_and_final_newlines() {
    let document = b"before\r\n```rust\r\n\r\nfn main() {}\r\n\r\n```\r\nafter";
    assert_eq!(
        sample_source::extract(document, "rust").unwrap(),
        b"\r\nfn main() {}\r\n\r\n"
    );
    assert_eq!(
        sample_source::extract(b"```rust\nx\n```", "rust").unwrap(),
        b"x\n"
    );
    assert_eq!(
        sample_source::extract(b"```python\nx\n```\n```rust\ny\n```\n", "rust").unwrap(),
        b"y\n"
    );
}

#[test]
fn malformed_or_ambiguous_fences_fail_closed() {
    for (source, error) in [
        ("nothing", "missing"),
        ("```rust\n```\n", "empty"),
        ("```rust\n \t\r\n```", "empty"),
        ("```rust\nx", "unclosed"),
        ("```rust\nx\n```\n```rust\ny\n```", "duplicate"),
        ("```rust\nx\n```\n```rust\ny", "duplicate"),
        ("```python\n```rust\nx\n```\n", "missing"),
    ] {
        assert!(
            sample_source::extract(source.as_bytes(), "rust")
                .unwrap_err()
                .contains(error),
            "{source:?}"
        );
        let toml = source.replace("rust", "toml");
        assert!(
            sample_source::extract(toml.as_bytes(), "toml")
                .unwrap_err()
                .contains(error),
            "{toml:?}"
        );
    }
}

fn dependencies(source: &[u8]) -> Result<(), String> {
    let mut parsed: toml::Value =
        toml::from_str(std::str::from_utf8(source).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let expected: toml::Value = toml::from_str(
        r#"
[dependencies]
bacnet-client = "=0.14.0"
bacnet-types = "=0.14.0"
bacnet-encoding = "=0.14.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
"#,
    )
    .unwrap();
    if let Some(features) = parsed
        .get_mut("dependencies")
        .and_then(|v| v.get_mut("tokio"))
        .and_then(|v| v.get_mut("features"))
        .and_then(toml::Value::as_array_mut)
    {
        features.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    if parsed != expected {
        return Err(format!("release dependency contract drifted: {parsed}"));
    }
    Ok(())
}

#[test]
fn dependency_contract_matches_the_documented_release() {
    dependencies(sample_source::extract(GUIDE, "toml").unwrap()).unwrap();
    let prose = std::str::from_utf8(GUIDE).unwrap();
    assert!(prose.contains("These instructions target **v0.14.0**"));
    assert!(prose.contains("minimum Rust version of **1.93**"));
}

#[test]
fn dependency_drift_is_detected_semantically() {
    let source = std::str::from_utf8(sample_source::extract(GUIDE, "toml").unwrap()).unwrap();
    for bad in [
        source.replace("=0.14.0", "0.14"),
        source.replace("rt-multi-thread", "rt"),
        format!("{source}unexpected = \"1\"\n"),
        source.replace("bacnet-encoding = \"=0.14.0\"\n", ""),
    ] {
        assert!(dependencies(bad.as_bytes()).is_err(), "{bad}");
    }
    dependencies(
        source
            .replace(
                "[\"macros\", \"rt-multi-thread\"]",
                "[\"rt-multi-thread\", \"macros\"]",
            )
            .as_bytes(),
    )
    .unwrap();
    dependencies(format!("# Formatting and comments are not semantic drift.\n{source}").as_bytes())
        .unwrap();
}
