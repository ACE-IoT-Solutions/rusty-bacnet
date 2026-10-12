---
section: Changed
---
- **Wire:** A B/IP foreign device holds up to 16 broadcasts until its first
  registration attempt after a start is answered or times out, then sends
  them in order, or drops them if the BBMD refuses it; a broadcast past that
  hold fails with an error (#1622).
