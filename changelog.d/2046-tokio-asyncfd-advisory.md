---
section: Security
---
- The workspace now requires tokio 1.53.3 or later, which fixes
  RUSTSEC-2026-0386 (`AsyncFd` I/O safety), and registers its serial and
  Ethernet descriptors through tokio's new unsafe `AsyncFd::register` with
  their safety argued in `bacnet_transport::sys` (#2046).
