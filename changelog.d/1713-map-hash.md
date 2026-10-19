---
section: Added
---
- `bacnet_types::map_hash`: a cheap multiply-and-rotate hasher for maps
  that remote input can't grow, which the client's TSM and the server's
  request admission now use (#1713).
