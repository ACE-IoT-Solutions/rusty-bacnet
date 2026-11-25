---
section: Fixed
---
- The ACE private index receives the same source archive and selected Python wheels as the staged GitHub release. Publication verifies the remote version tag, tested commit and artifact hashes immediately before upload. Source lock repair and archive builds use the checkout’s Rust toolchain.
