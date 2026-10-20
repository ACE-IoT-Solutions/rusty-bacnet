---
section: Added
---
- A B/IP node can join an IPv4 multicast group on an explicit interface
  (Annex J.8): its broadcasts go to the group and it takes broadcasts only
  from the group (`BipTransport::set_multicast`, `.multicast(...)` on the
  client, server and endpoint builders, #941).
