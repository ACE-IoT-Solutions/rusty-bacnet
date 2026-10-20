---
section: Migration notes
---
- **MS/TP counters (Rust API, #1897):** read `MstpCounters::late_deadlines`
  where you read `deadline_misses`, in `bacnet_mstp_core` and
  `bacnet_transport::mstp`. `bacnet_mstp_core::LATE_DEADLINE` is gone: set
  `MstpCoreConfig::late_deadline` instead (`DEFAULT_LATE_DEADLINE`, 1 ms). The
  Tokio transport counts from 5 ms (`HOST_LATE_DEADLINE`).
