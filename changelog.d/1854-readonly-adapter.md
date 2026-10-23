---
section: Fixed
---
- **Breaking (Rust API):** Passive MS/TP serial monitoring uses native read-only access and awaited release. Failed cleanup blocks CLI replacement, cancelled stops retain progress, and Unix preserves inherited hangup policy (#1854).
