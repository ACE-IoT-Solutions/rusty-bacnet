---
section: Fixed
---
- **Wire:** The client acknowledges the first segment of a response
  immediately, so property reads, trend log pages, and file reads complete with
  peers that wait for this acknowledgment before sending the rest of a window.
