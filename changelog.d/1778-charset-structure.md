---
section: Changed
---
- `CharacterStringRef::decode` refuses what the owned path refuses: a DBCS
  string without its code page and malformed UCS-2 or UCS-4. Both run one
  shared check, and the owned validator now reports its faults at offsets
  in the data with the view's wording (#1778).
