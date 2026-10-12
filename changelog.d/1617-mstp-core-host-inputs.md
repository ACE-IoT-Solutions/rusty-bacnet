---
section: Added
---
- **Breaking (Rust API):** `bacnet-mstp-core` takes invalid frames from a
  host frame assembler (`on_invalid_frame`), ignores all host input during
  its own transmissions as echo, and counts frames sent from its own
  address in `own_address_frames` (#1617).
