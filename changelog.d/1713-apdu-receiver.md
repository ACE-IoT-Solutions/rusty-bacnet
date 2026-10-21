---
section: Changed
---
- **Breaking (Rust API):** `NetworkLayer::start` returns an `ApduReceiver`
  that decodes the transport's NPDUs as it is polled, with no dispatch task
  or second queue in between; network controls flow only while it is
  polled, and the transport's queue bounds overload (#1713).
