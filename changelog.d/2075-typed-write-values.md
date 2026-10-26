---
section: Changed
---
- `bacnet write` sends a number as the property's type: ENUMERATED for an enumeration, REAL, Double, INTEGER or Unsigned for an analog or integer value. Bare dates and times now parse, and `date:`/`time:` refuse out-of-range fields instead of wrapping (#2075).
