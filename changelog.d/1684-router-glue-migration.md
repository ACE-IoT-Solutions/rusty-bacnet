---
section: Migration notes
---
- **Router glue (Rust API, #1684):** drain a `BipPort`'s or `BbmdPort`'s
  counts with `take_counters()` (`counters()` looks) for `refused()`.
  `HeldBroadcast` and `BipPort` take a slot length `N`, `MAX_NPDU` by
  default (`<HeldBroadcast>::EMPTY` names it); `PortError::HeldStorage` and
  `TargetStorage` carry `needed`, `available`, and short slots are
  `HeldLength`. `bip::Ingress` is exhaustive; `mstp::BROADCAST` is the
  codec's `BROADCAST_MAC`.
