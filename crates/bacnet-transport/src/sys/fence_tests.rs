//! The fence around `sys` (#1923): no Rust source of this crate outside
//! `src/sys/` may hold unsafe code, nor allow the `unsafe_code` lint. The
//! lint alone can't hold the line, since a new `#[allow(unsafe_code)]`
//! anywhere silences it; this test fails on one.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, skipping `skip` and build output.
fn rust_files(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == skip || path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        if path.is_dir() {
            rust_files(&path, skip, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `code` (one line, comments removed) holds the keyword `unsafe`
/// as a word of its own, or names the `unsafe_code` lint in anything but a
/// `deny` or `forbid`.
fn holds_unsafe(code: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let keyword = code.match_indices("unsafe").any(|(at, found)| {
        let before = code[..at].chars().next_back();
        let after = code[at + found.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    });
    let lint = code.contains("unsafe_code") && !code.contains("deny(") && !code.contains("forbid(");
    keyword || lint
}

/// The line's code, without a `//` comment (doc comments included).
fn code_of(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

#[test]
fn unsafe_stays_in_sys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sys = root.join("src").join("sys");
    let mut files = Vec::new();
    rust_files(root, &sys, &mut files);
    assert!(files.len() > 50, "found only {} files", files.len());
    let offences: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            text.lines()
                .enumerate()
                .filter(|(_, line)| holds_unsafe(code_of(line)))
                .map(|(number, line)| format!("{}:{}: {}", file.display(), number + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offences.is_empty(),
        "unsafe code belongs in src/sys/ (#1923):\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_fence_sees_unsafe_and_lets_prose_by() {
    for caught in [
        "let x = unsafe { f() };",
        "unsafe fn f() {}",
        "unsafe impl Send for T {}",
        "unsafe extern \"C\" {",
        "#[unsafe(no_mangle)]",
        "#[allow(unsafe_code)]",
        "#![allow(unsafe_code)]",
        "#[cfg_attr(test, allow(unsafe_code))]",
        "#[expect(unsafe_code)]",
    ] {
        assert!(holds_unsafe(code_of(caught)), "{caught}");
    }
    for passed in [
        "#![forbid(unsafe_code)]",
        "#![deny(unsafe_code)]",
        "/// Unsafe code lives in sys; no unsafe here.",
        "let x = 1; // unsafe { } in a comment",
        "#[deny(unsafe_op_in_unsafe_fn)]",
        "let unsafe_count = 0;",
    ] {
        assert!(!holds_unsafe(code_of(passed)), "{passed}");
    }
}
