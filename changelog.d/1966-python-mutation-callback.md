---
section: Added
---
- **Python API:** `BACnetServer(mutation_policy=...)` also takes a callable that
  gets a redacted `MutationContext` and decides each covered mutation;
  exceptions and non-bool answers deny. Its runtime signature default
  now reads `...` (#1966).
