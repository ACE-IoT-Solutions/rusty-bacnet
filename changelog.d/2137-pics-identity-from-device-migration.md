---
section: Migration notes
---
- **PICS (Rust API, #2137):** Remove `vendor_name`, `model_name`,
  `firmware_revision` and `application_software_version` from your
  `PicsConfig`, and set them on your Device object instead; `generate_pics`
  reads them from the Device when you call it. Without a Device they print
  empty. The standalone generator's vendor ID is now the Device's
  Vendor_Identifier, falling back to the `ServerConfig` identity's.
