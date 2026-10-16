---
section: Added
---
- Client builders can return one shared type for B/IP, B/IPv6 and BACnet/SC,
  retaining their configured options and SC reconnect policy. Existing concrete
  builds remain available; SC rejects invalid segment windows before dialing
  (#1014).
