---
section: Changed
---
- **CI:** A merge to `dev` that has its PR head's tree, where `CI OK` and
  `Native OK` passed, skips re-running the same jobs, unless it changes a
  cache-key input (#1466).
