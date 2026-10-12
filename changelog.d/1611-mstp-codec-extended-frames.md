---
section: Added
---
- `bacnet-mstp-codec` encodes and decodes extended MS/TP frames (Types 32
  and 33, COBS-encoded with a CRC-32K) for NPDUs too long for a standard
  frame, with `decode_frame_in_place` and storage constants per frame class.
  The MS/TP core doesn't take them yet (#1611).
