---
section: Migration notes
---
- **Element table (Rust and Python API, #1888):** the (object type,
  property) table of constructed elements moved from the Python binding to
  `bacnet_encoding::constructed::property_element`, with nothing to change
  for existing callers. Python reads of Event_Algorithm_Inhibit_Ref now
  have `.tag` `"object_property_reference"` and a mapping of
  `object_identifier`, `property_identifier` and `property_array_index` as
  `.value`; `.octets` still gives the octets.
