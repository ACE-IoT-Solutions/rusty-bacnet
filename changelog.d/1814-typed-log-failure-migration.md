---
section: Migration notes
---
- **Log record failures (Rust API, #1814):** `LogDatum::Failure` and
  `LogValue::Failure` hold `error_class: ErrorClass` and
  `error_code: ErrorCode` instead of `u32`s. Build them from the constants
  (`ErrorClass::PROPERTY`) or `from_raw`, and read the numbers with
  `to_raw`. Decoding a record whose failure class or code is wider than 16
  bits now fails instead of keeping the value.
