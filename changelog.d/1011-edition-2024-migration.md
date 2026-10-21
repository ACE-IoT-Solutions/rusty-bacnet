---
section: Migration notes
---
- **Rust toolchain (D8, #1011):** Upgrade Rust and Cargo to 1.95 or newer before
  building 0.15 from source. Consumers can keep their own Rust edition. The
  minimum compiler increases only in minor releases, to a stable Rust
  release at least about six months old.
