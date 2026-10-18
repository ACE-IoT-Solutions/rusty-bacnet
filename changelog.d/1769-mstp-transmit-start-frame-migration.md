---
section: Migration notes
---
- **MS/TP transmit (Rust API, #1769):** match
  `Action::Transmit { id, header, length }`; right before enabling the line
  driver call `on_transmit_started(id, now)`, send the octets in its
  `Ok(frame)`, and send nothing on `Err` (a deferred frame comes back as a
  new `Transmit` action). Stop copying the frame at the poll.
