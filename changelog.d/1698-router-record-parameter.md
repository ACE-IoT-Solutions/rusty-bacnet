---
section: Changed
---
- `bacnet-router-core`'s `Storage`, `Router` and `RouterIn` take a trailing const parameter `B`, defaulting to 32: the global broadcasts kept for duplicate suppression, 0 for none (#1698).
