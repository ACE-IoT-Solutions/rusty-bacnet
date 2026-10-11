---
section: Migration notes
---
- **MS/TP core (Rust API, #1625):** keep the receiver on, and drain it into
  the core, until `on_transmit_started` succeeds; send nothing on
  `TransmitDeferred`, as on any start error, and drop an unstarted frame
  when the core queues `SetDirection(Receive)`.
