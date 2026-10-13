---
section: Added
---
- New `bacnet-mstp-core` crate: a sans-IO `no_std` MS/TP master node
  (Clause 9) on caller-supplied time, with bounded queues, typed actions for
  an adapter and optional extended frames; `alloc` adds heap-backed queues.
  The MS/TP transport runs on it. By Andrew Rodgers (ACE IoT Solutions)
  (#1605).
