use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const LEDGER_JSON: &str =
    include_str!("../../../../docs/conformance/bacnet-135-2020.json");
pub(super) const SUPPORT_SUMMARY: &str =
    include_str!("../../../../docs/conformance/support-summary.md");
pub(super) const PICS_DRAFT: &str = include_str!("../../../../docs/conformance/pics-draft.md");
pub(super) const BIBBS_DRAFT: &str = include_str!("../../../../docs/conformance/bibbs-draft.md");
pub(super) const STANDARD_LEDGER: &str =
    include_str!("../../../../docs/conformance/standard-135-2020-ledger.md");

pub(super) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(super) fn repo_path(path: &str) -> PathBuf {
    repo_root().join(path)
}

pub(super) fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
}

pub(super) fn standard_ledger() -> Cow<'static, str> {
    normalize_line_endings(STANDARD_LEDGER)
}

pub(super) fn read_repo_file(path: &str) -> String {
    let body = fs::read_to_string(repo_path(path)).expect("repo file should be readable");
    normalize_line_endings(&body).into_owned()
}
