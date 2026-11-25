---
section: Fixed
---
- Truncated ReadProperty, ReadPropertyMultiple, ReadRange and decoded AtomicReadFile acknowledgments report a structured missing-byte error when the parser identifies the complete service payload's end. The diagnostic includes the required and available sizes.
