---
section: Changed
---
- **Breaking (Rust API, Python API):** An empty DeviceCommunicationControl or
  ReinitializeDevice password is refused when sending or configuring, and one
  received from a peer counts as no password (#2094).
