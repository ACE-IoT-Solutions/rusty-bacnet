---
section: Changed
---
- All of `bacnet-transport`'s remaining unsafe OS calls live in one private
  `sys` module behind safe wrappers; the rest of the crate has no unsafe code,
  and a test fails if any appears outside `sys` (#1923).
