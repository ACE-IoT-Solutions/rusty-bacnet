---
section: Changed
---
- Every crate without unsafe code now forbids it, the workspace lints
  require a safety comment on each remaining unsafe block and a safety
  section on each unsafe function, and the Windows UDP control-message
  parser bounds-checks without arithmetic that could wrap (#1880).
