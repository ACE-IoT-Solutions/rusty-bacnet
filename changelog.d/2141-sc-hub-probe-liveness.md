---
section: Fixed
---
- **Wire:** The SC hub no longer retires a connector just for not answering its
  probes; it waits until nothing has arrived since the probe for 360 seconds
  (`ScHubProbePolicy::with_liveness_bound`), so a dead connector now lingers
  about 8 minutes instead of 65–95 seconds (#2141).
