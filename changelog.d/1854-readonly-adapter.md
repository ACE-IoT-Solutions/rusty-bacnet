---
section: Fixed
---
- **Breaking (Rust API):** The passive MS/TP monitor opens serial devices with native read-only access and awaits ownership release on stop. Cancelled stops retain their cleanup state (#1854).
