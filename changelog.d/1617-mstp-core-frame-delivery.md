---
section: Added
---
- **Breaking (Rust API):** `bacnet-mstp-core` passes received Test_Response
  and proprietary frames up as the new `Action::DeliverFrame`, counted in
  `frames_delivered`, instead of dropping them (#1617).
