---
section: Added
---
- BACnet/SC dials lint this node's certificate, key and CA file first: hard
  failures stop the dial before any socket, and warnings (expiry within 30 days,
  no direct issuer in the CA file) need `y` in the TUI. The SC screen gains a
  My credentials view (#968).
