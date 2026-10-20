---
section: Fixed
---
- The Ethernet transport counts and logs XID and TEST replies that fail to
  go out, exposed as `EthernetTransport::reply_send_failures`, and a send
  that puts only part of a frame on the wire is now an error (#1918).
