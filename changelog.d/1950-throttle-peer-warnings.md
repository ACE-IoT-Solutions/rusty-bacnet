---
section: Changed
---
- Warnings a peer can set off once per packet or connection (undecodable
  APDUs in the client and server, refused segments, bad SC direct and hub
  frames, a full Ethernet NPDU channel) warn at most once per 10 seconds with
  a count; the rest log at debug (#1950).
