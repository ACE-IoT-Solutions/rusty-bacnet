---
section: Migration notes
---
- **SC admission (Rust API, #518):** `ScNpduAdmissionPolicy`, `ScNpduDropCounts`
  and the new `ScControlRateLimit` are `#[non_exhaustive]`; construct them via
  `Default` or the builders. An SC node now drops a source's network-layer
  messages beyond 256 at once or 32 a second; a site with a router fronting many
  networks raises it with `with_control_rate_limit()`.
