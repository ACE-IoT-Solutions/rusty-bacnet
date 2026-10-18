---
section: Changed
---
- Confirmed requests no longer create a channel for their timer progress;
  the progress now rides in the transaction's existing shared state, saving
  about 1,100 instructions per request (#1727).
