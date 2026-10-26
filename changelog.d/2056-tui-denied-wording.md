---
section: Changed
---
- The TUI shows a request the device refuses as denied by the device's
  policy, without calling it a write, so a denied DeviceCommunicationControl
  or ReinitializeDevice reads right (#2056).
