---
section: Changed
---
- Every crate without unsafe code forbids it, bacnet-encoding's tests too;
  each remaining unsafe block needs a safety comment, and
  `bacnet-transport`'s OS calls sit in one private `sys` module behind safe
  wrappers. The Windows UDP control-message parser can no longer wrap
  (#1880, #1881, #1923).
