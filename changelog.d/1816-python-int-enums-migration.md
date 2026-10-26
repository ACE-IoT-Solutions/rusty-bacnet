---
section: Migration notes
---
- **Python enums (#1816):** `ErrorClass` and `ErrorCode` members are now ints,
  so `str()` gives the number (`repr()` still names it) and an `(ErrorClass,
  ErrorCode)` argument also takes plain ints. A value without a name is a
  pseudo-member whose `name` is None; a number outside the field raises
  `ValueError` instead of `OverflowError`.
