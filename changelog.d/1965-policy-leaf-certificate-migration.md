---
section: Migration notes
---
- **Transport provenance (Rust API, #1965):** `TransportProvenance` now holds
  the verified leaf certificate, so it is `Clone` but no longer `Copy`, and its
  methods take `&self`; clone it where a copy was implied. `ScHubAdmissionInput`,
  which carries one, is no longer `Copy` either.
