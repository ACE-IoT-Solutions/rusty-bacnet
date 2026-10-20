---
section: Migration notes
---
- **SC admission (Rust API, #518):** `ScNpduAdmissionPolicy` gains
  `control_rate` and `ScNpduDropCounts` gains `control_rate_drops`; build
  either with `..Default::default()`. An SC node now drops a source's
  network-layer messages beyond 32 at once or 8 a second; raise the limit with
  `with_control_rate_limit()` if a router needs more.
