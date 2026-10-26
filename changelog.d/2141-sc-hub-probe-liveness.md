---
section: Fixed
---
- **Wire:** The SC hub no longer retires a connector just for not answering its
  probes; it waits until nothing has arrived since the probe for 360 seconds,
  set with `ScHubProbePolicy::with_liveness_bound` or Python's
  `probe_liveness_bound_ms` (#2141).
