//! The hosted B/IP receive path, management exchanges and registration
//! worker as they were before the node engine (#1622), kept for the
//! cutover's differential test only ([`super::differential_tests`]). Deleted
//! with that test before the cutover merges.

pub(super) mod client;
pub(super) mod receive;
