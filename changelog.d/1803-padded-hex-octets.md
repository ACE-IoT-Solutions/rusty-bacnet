---
section: Changed
---
- Decoding errors gain a `{:#04X}` placeholder for octets (#1803): the B/IP,
  B/IPv6 and Ethernet frame errors print octets as `0x0B` again, and an
  unknown BACnet/SC BVLC-Result code prints upper case (`0x0B`, not `0x0b`).
