---
section: Migration notes
---
- **bacnet-bip-codec (Rust API, #1684):** `BvllError` and `Bvll6Error`
  are `#[non_exhaustive]`: give a `match` on either a wildcard arm.
