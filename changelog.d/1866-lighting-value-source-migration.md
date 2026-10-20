---
section: Migration notes
---
- **Lighting outputs (Rust API, #1866):** `write_property` refuses a
  Present_Value write on Lighting Output and Binary Lighting Output, and a
  Lighting_Command write on Lighting Output, with WRITE_ACCESS_DENIED. Call
  `write_property_from` with a `CommandOrigin`, as for the other commandable
  objects, or `LightingOutputObject::set_lighting_command` for an
  application command.
