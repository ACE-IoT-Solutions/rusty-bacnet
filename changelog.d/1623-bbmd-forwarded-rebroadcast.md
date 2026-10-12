---
section: Fixed
---
- **Wire:** a BBMD decides whether to rebroadcast a peer's Forwarded-NPDU on
  its subnet from its own BDT row, as Annex J.4.5 says, not the sender's: it
  does so when its own row is two-hop and the datagram came by unicast from
  off its subnet (#1623).
