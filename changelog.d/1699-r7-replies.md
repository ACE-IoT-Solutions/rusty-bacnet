---
section: Fixed
---
- Router-local replies now identify the addressed endpoint while returning through the captured ingress port and peer. Segmented responses, cached APDU replay, MS/TP handoff and direct BACnet/SC replies preserve that context and its owner lifetimes (#1699).
