---
section: Fixed
---
- **Breaking (Rust API):** Router shutdown awaits transport cleanup and reports failures. Cancelled startup and shutdown retain cleanup ownership; dropping a router initiates cleanup (#2002).
