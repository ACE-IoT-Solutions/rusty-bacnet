---
section: Changed
---
- **MS/TP transport:** every host port now waits the longest T_usage_timeout
  (35 ms) and T_reply_timeout (300 ms) Clause 9.5.3 allows, set by
  `MstpConfig::usage_timeout` and `reply_timeout`, so a successor, poll
  answer or reply that a USB adapter delivers late is not missed (#1655).
