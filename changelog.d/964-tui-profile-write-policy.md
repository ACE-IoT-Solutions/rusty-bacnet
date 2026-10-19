---
section: Changed
---
- New `bacnet tui` profiles default to `writes = "session"`, and
  `dangerous = true` is accepted with it; raising a profile's write policy
  asks for its authorization attestation again (#964).
