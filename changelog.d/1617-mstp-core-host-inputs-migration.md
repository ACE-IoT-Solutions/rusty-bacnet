---
section: Migration notes
---
- **MS/TP core (Rust API, #1617):** handle `Action::DeliverFrame` where you
  match on `Action`. A host adapter that assembles frames reports bad ones
  with `on_invalid_frame`, drops its partial frame when a transmit start is
  confirmed, and must not feed its own echo back after the transmission
  completes.
