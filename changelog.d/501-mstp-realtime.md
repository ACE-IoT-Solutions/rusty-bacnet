---
section: Added
---
- **Breaking (Rust API):** On Linux the MS/TP transport's dedicated thread can
  run under `SCHED_FIFO` and be pinned to CPUs (`MstpThreadConfig`, CLI
  `--mstp-rt-priority` and `--mstp-cpus`); `start` fails if either can't be
  applied, and `execution_status()` reports the outcome (#501).
