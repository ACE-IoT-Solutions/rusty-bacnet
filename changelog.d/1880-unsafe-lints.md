---
section: Changed
---
- Every crate without unsafe code now forbids it, and the workspace lints
  require a safety comment on each remaining unsafe block and a safety
  section on each unsafe function (#1880).
