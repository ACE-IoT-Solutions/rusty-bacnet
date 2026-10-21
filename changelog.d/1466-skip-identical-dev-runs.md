---
section: Changed
---
- **CI:** A merge to `dev` that could have been a fast-forward to a PR head
  whose runs passed `CI OK` and `Native OK` skips re-running the same jobs,
  unless it changes a cache-key input (#1466).
