---
section: Migration notes
---
- **Typed single reads (Python API, #1499):** compare these reads by `.tag`
  and `.value` (a shed level is `{"kind", "value"}`, a factor
  `(format_type, format_class, value)`); `__reduce__()[1][1]` gives the
  octets. See [typed constructed values](docs/python-api.md#typed-constructed-values).
