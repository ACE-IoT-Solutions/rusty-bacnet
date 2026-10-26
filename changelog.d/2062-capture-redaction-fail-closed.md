---
section: Security
---
- The TUI capture masks DeviceCommunicationControl and ReinitializeDevice
  passwords in every frame it keeps, including requests this node receives
  and damaged frames it overhears, failing closed when a frame can't be read
  cleanly (#2062).
