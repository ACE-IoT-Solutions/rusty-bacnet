---
section: Added
---
- New `bacnet-mstp-core` crate: a sans-IO `no_std` MS/TP master node
  (Clause 9) with no allocator, on caller-supplied time, with bounded
  queues, typed actions for a platform adapter and optional extended
  frames. The MS/TP transport runs on it. Contributed by Andrew Rodgers
  (ACE IoT Solutions) (#1605).
