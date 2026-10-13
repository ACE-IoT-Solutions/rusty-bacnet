---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` settles its API before it first
  publishes: `#[non_exhaustive]` types with constructors, a
  `MstpCoreConfig::STANDARD` preset, the other cores' names, drained
  counters, an early `now` taken as the latest, and its codec re-exported
  as `codec` (#1684).
