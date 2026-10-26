---
section: Fixed
---
- **Wire:** A DateTime Value or DateTime Pattern Value now sends each set
  Priority_Array slot as one date-time element, so a whole read gives 16
  slots; the CLI, TUI and Python show that slot as its date and time (#2098).
