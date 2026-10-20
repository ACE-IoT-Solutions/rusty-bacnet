---
section: Changed
---
- The TUI sizes its name-read ReadPropertyMultiple batches from the device's
  Max_APDU and the client's own limit, so a small device's answer fits
  without a failed request first and a large one takes more names a
  request (#1930).
