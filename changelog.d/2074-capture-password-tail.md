---
section: Security
---
- Captures mask a DeviceCommunicationControl or ReinitializeDevice password
  through the end of the request, so a password tag that claims too short a
  length no longer leaves the rest of the password in the ring, the hex view
  or a pcap export (#2074).
