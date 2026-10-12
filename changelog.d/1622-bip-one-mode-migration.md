---
section: Migration notes
---
- **B/IP modes (Rust API, #1622):** A `BipTransport` can't call both
  `enable_bbmd` and `register_as_foreign_device`: `start()` fails. Run the
  BBMD and the foreign device on separate transports (another port or
  interface), or drop the one you don't need.
