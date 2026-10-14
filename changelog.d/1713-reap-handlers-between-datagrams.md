---
section: Changed
---
- The server joins finished request handlers between datagrams instead of
  waking its dispatch loop as each one ends: a B/IP ReadProperty round trip
  is 2 to 8 microseconds shorter, and one client gets 15% (macOS) to 130%
  (Linux) more requests a second (#1713).
