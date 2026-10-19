---
section: Added
---
- The MS/TP simulator models each master's baud and protocol-clock error
  within the one percent Clause 9.2.3 allows; scenarios at the edges check
  token, reply and usage timing. Below 38,400 baud, edge clocks can lock
  step without timer jitter (#1772, #1804).
