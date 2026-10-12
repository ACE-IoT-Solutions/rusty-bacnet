---
section: Changed
---
- A B/IP transport's own management requests (`read_bdt` and the other BBMD
  helpers, and a foreign device's registration) run side by side, one per
  peer and operation, instead of one at a time; only the same request to the
  same peer, or a second one a success answers, is busy (#1622).
