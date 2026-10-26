---
section: Migration notes
---
- **PICS (Rust API, #2137):** Remove `vendor_name`, `model_name`,
  `firmware_revision` and `application_software_version` from your
  `PicsConfig`, and set them on your Device object instead; `generate_pics`
  reads them from the Device when you call it. Without a Device they print
  empty. If the PICS needs different text, edit the generated document.
