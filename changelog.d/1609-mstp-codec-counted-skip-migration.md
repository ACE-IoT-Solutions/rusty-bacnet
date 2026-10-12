---
section: Migration notes
---
- **MS/TP stream decoder (Rust API, #1609):** `DecodeEvent::Malformed`'s
  `discard` can exceed what is buffered when a frame is skipped by count,
  `Invalid` discards a whole run of buffered noise, a refusal's
  `BufferExhausted.needed` leaves out the octets of a skip, and `phase()`
  is `Data` while a skip lasts. Track stream offsets by those rules.
