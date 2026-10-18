---
section: Changed
---
- **Breaking (Rust API):** `AnyTransport` now requires a fallback in downstream
  matches, allowing callers to handle additional transport variants enabled by
  dependency features (#1725).
