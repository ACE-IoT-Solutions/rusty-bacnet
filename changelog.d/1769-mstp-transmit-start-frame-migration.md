---
section: Migration notes
---
- **MS/TP transmit (Rust API, #1769):** match
  `Action::Transmit { id, header, length }` and take the octets to send from
  `let frame = core.on_transmit_started(id, now)?`, right before enabling the
  line driver; stop copying the frame at the poll.
