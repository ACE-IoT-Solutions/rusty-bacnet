---
section: Fixed
---
- **Breaking (Rust API):** A generated PICS takes Protocol_Version and
  Protocol_Revision from the Device it describes, so it no longer claims
  revision 24 while the Device reports 22; `PicsConfig` loses its two
  protocol fields, and `bacnet_types` gains the shared constants (#2125).
