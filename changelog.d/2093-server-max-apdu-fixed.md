---
section: Changed
---
- A server keeps the Max_APDU_Length_Accepted it resolved at start when its
  transport's receive capacity later falls; replies still fit each route. The
  Rust API guide now says so (#2093).
