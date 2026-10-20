---
section: Changed
---
- A flood of malformed NPDUs no longer floods the log: the network layer warns
  about dropped inbound NPDUs at most once per interval, with the count since
  the last warning, and logs the rest at debug (#1724).
