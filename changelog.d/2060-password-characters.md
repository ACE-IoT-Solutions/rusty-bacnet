---
section: Fixed
---
- **Wire:** DeviceCommunicationControl and ReinitializeDevice count the
  20-character password limit in characters, not octets, so a server accepts a
  valid non-ASCII password longer than 20 octets (#2060).
