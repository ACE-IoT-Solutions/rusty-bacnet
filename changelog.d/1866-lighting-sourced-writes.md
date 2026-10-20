---
section: Changed
---
- **Breaking (Rust API):** Lighting Output and Binary Lighting Output take
  Present_Value and Lighting_Command writes only through `write_property_from`,
  which names the writer their Value_Source records (#1866).
