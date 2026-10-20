---
section: Migration notes
---
- **Lighting outputs (Rust API, #1866):** `write_property` refuses
  Present_Value and Value_Source writes on Lighting Output and Binary
  Lighting Output, and Lighting_Command writes on Lighting Output. Call
  `write_property_from` with a `CommandOrigin`, as for the other commandable
  objects; in a server, `BACnetServer::write_local` with a
  `LocalCommandSource`. `LightingOutputObject::set_lighting_command` still
  takes an application command, with source NONE.
