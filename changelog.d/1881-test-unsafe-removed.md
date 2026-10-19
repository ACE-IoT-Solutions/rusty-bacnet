---
section: Changed
---
- The tests' pseudo-terminal, rlimit, interface-index and IPv6 packet-info
  calls go through nix instead of raw libc, the three counting allocators are
  one shared test module, and bacnet-encoding now forbids unsafe code in its
  tests too (#1881).
