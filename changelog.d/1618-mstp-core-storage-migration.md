---
section: Migration notes
---
- **MS/TP core (Rust API, #1618):** pass `&mut ActionBuffers<N>` to
  `MasterCore::new` after the receive storage; queues may now be lent as
  `&mut`. `CoreError::ActionCapacity` is gone, starting a transmit before
  polling its frame returns `TransmitNotPolled`, and
  `MstpCounters::action_drops` is now `delivery_drops`.
