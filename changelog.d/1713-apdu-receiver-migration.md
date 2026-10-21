---
section: Migration notes
---
- **NetworkLayer (Rust API, #1713):** `NetworkLayer::start` returns
  `bacnet_network::layer::ApduReceiver` (`recv`, `poll_recv`, `try_recv`,
  `close`, `is_closed`) instead of `mpsc::Receiver<ReceivedApdu>`: `recv_many`,
  `poll_recv_many`, `blocking_recv`, `len`, `is_empty`, `capacity` and
  wrapping it in `ReceiverStream` are gone. Keep polling it while you need
  network controls, or use `start_with_admission`; `stop` ends its stream.
