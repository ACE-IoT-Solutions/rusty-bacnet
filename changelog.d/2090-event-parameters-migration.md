---
section: Migration notes
---
- **Event_Parameters (Rust and Python API, #2090):** match
  `BACnetEventParameter::None` instead of `Opaque { tag: 20, .. }`, and the new
  variants instead of `Opaque` for tags 3, 8, 10, 11, 13 to 18, 21 and 22;
  `Opaque` now carries only unassigned tags. `Datum::Constructed` has a
  `fields` member. In Python those algorithms read as their own `kind`, not
  `other`.
