---
section: Changed
---
- Some codec error messages read differently (#1646, #1776, #1777, #1803):
  a context is followed by a colon, nested faults name their label, encoder
  range checks begin `value out of range:`, invalid UTF-8 says how many
  octets were valid, and an unknown BACnet/SC BVLC-Result code prints in
  upper case.
