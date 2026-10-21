---
section: Migration notes
---
- **Typed single reads (Python API, #1499, #1888):** compare these reads by
  `.tag` and `.value` (a shed level is `{"kind", "value"}`, a factor
  `(format_type, format_class, value)`, Event_Algorithm_Inhibit_Ref an
  `object_property_reference` mapping of `object_identifier`,
  `property_identifier` and `property_array_index`); the new `.octets` gives
  the octets. See [typed constructed values](docs/python-api.md#typed-constructed-values).
