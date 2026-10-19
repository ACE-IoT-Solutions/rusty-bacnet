---
section: Added
---
- B/IP, B/IPv6, BACnet/SC and MS/TP transports have a bounded, lossy frame
  tap, `frame_tap()`, that shows every link-layer frame sent and received,
  control traffic included, with no copying while nobody subscribes
  ([docs](docs/rust-api.md#frame-tap), #957).
