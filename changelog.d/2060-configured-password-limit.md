---
section: Changed
---
- **Breaking (Rust API, Python API):** A server or endpoint refuses a
  configured DCC or ReinitializeDevice password over 20 characters, which no
  request could carry: at start in Rust, with ValueError at construction in
  Python (#2060).
