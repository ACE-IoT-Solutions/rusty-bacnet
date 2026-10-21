---
section: Added
---
- `bacnet tui` keeps named B/IP and BACnet/SC profiles (hub URLs, credential
  paths but never their contents, a UUID made once), with saved
  first-use authorization and connect, disconnect and retry; it refuses a
  missing credential file before sending anything. Flags-only runs save
  nothing (#962).
