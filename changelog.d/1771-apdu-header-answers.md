---
section: Added
---
- The allocation-free `ApduHeader` also reads and writes an Error's header
  and a whole Reject or Abort, so firmware without a heap can refuse a
  request it doesn't serve (#1771).
