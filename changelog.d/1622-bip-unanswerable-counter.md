---
section: Changed
---
- **Breaking (Rust API):** B/IP `ReceiveCounters::unanswerable_request`
  counts the requests dropped unanswered for their source, and
  `ReceiveCounters` is now `#[non_exhaustive]`: read its fields, or start
  from `Default` (#1622).
