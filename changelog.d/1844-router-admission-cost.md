---
section: Changed
---
- The router core's admission check builds one arrival record per frame, which
  trims the firmware router image, and the hosted router hands its own network
  controls over without decoding the frame a second time (#1844).
