---
section: Migration notes
---
- **Decoded returns (#2080):** Rust and Python `create_object` return an
  object identifier, not the ACK octets. The CLI's `create-object` prints
  `Created TYPE:instance`, with `object_type` and `instance` in JSON. Python
  `atomic_read_file`/`atomic_write_file` return dicts. Bit strings, including
  `result_flags`, are `BitString`: read `.unused_bits`, `.data` or `.bits`.
