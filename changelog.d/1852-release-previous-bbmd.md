---
section: Added
---
- A B/IP foreign device moved to another BBMD through its Network Port can ask the old BBMD to delete its entry at once (`BipTransport::set_release_previous_bbmd`, off by default); a NAK or silence is counted and never fails the move (#1852).
