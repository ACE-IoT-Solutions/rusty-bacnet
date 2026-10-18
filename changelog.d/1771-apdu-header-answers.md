---
section: Added
---
- **Breaking (Rust API):** the allocation-free `ApduHeader` also reads and
  writes an Error's header and a whole Reject or Abort, so firmware without a
  heap can refuse a request it doesn't serve; the enum is `#[non_exhaustive]`
  (#1771).
