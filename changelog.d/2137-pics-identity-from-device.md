---
section: Fixed
---
- **Breaking (Rust API):** A generated PICS takes Vendor_Name, Model_Name,
  Firmware_Revision and Application_Software_Version from the Device it
  describes, so it can't print blank or stale product details; `PicsConfig`
  loses its four product fields (#2137).
