---
section: Added
---
- **Rust API:** `BACnetNameValue` in `bacnet_types::constructed`, a tag's
  name and at most one primitive value, so a date and a time are two tags,
  with `encode_name_value` and `decode_name_value` in
  `bacnet_encoding::constructed` (#1553, #1583).
