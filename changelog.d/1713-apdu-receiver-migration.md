---
section: Migration notes
---
- **NetworkLayer (Rust API, #1713):** `NetworkLayer::start` returns
  `bacnet_network::layer::ApduReceiver` (`recv`, `poll_recv`, `try_recv`,
  `close`) instead of `mpsc::Receiver<ReceivedApdu>`; update type names. Keep
  polling it while you need network controls, or use `start_with_admission`
  for a queue that fills on its own.
