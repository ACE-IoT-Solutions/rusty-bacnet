---
section: Security
---
- The TUI capture masks DeviceCommunicationControl and ReinitializeDevice
  passwords in damaged frames it overhears too, failing closed when a frame
  can't be read cleanly (#2062).
