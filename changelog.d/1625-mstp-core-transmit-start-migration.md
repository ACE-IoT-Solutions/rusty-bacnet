---
section: Migration notes
---
- **MS/TP core (Rust API, #1625, #1618):** keep the receiver on and feed
  the core until `on_transmit_started` succeeds; send nothing on
  `TransmitDeferred`, as on any start error, and drop an unstarted frame
  when the core queues `SetDirection(Receive)`.
  `MstpCounters::discarded_while_answering` is now `discarded_while_busy`.
