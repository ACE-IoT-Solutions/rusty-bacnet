---
section: Added
---
- **Wire:** With `MstpCoreConfig::extended_frames` on, the MS/TP core takes
  and sends extended frames (Types 32 and 33) for NPDUs of up to 1497
  octets and checks reserved COBS types. It is off by default, which leaves
  the core as it was (#1611).
