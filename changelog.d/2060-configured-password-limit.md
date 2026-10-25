---
section: Changed
---
- **Breaking (Rust API):** A server or endpoint, Python's included, refuses
  to start with a configured DCC or ReinitializeDevice password over 20
  characters, which no request could ever match (#2060).
