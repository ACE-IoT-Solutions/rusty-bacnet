---
section: Changed
---
- **Breaking (Rust API):** `bacnet-bip-core` settles its API before it first
  publishes: the types that may grow are `#[non_exhaustive]`, with
  constructors for its inputs, a few names follow the other cores, and it
  re-exports its codec as `codec` (#1684).
