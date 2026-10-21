---
section: Changed
---
- Warnings a peer can set off once per packet or connection (dropped inbound
  NPDUs, undecodable or refused requests and segments, bad SC direct and hub
  connections, a full Ethernet NPDU channel) warn at most once per 10
  seconds with a count; the rest log at debug (#1724, #1950).
