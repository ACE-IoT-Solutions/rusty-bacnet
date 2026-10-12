---
section: Fixed
---
- **Wire:** a BBMD rebroadcasts a peer's Forwarded-NPDU on its subnet when
  its own BDT row is two-hop and the datagram came by unicast, as Annex J.4.5
  says, not by the sender's row; never for a peer on its own subnet at its own
  port (#1623).
