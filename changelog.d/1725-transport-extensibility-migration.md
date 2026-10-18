---
section: Migration notes
---
- **AnyTransport (Rust API, #1725):** Add a `_` arm to matches outside
  `bacnet-transport`. Keep arms for the transports the caller supports and choose
  an appropriate fallback for other transports. Existing variants remain
  directly constructible.
