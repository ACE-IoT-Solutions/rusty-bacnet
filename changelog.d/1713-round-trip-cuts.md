---
section: Changed
---
- Requests cost less (#1713, #1727, #1728, #1729): the server runs and reaps
  request handlers more cheaply and encodes each reply once, and no task sits
  between transport and application. One B/IP ReadProperty client gets about
  36% more requests a second than on 0.13.0 ([Benchmarks](Benchmarks.md)).
