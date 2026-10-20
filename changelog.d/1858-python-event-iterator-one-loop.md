---
section: Changed
---
- **Python API:** event iterators complete concurrent reads one at a time.
  A read held up behind an item whose event loop stopped raises one
  `RuntimeWarning` after 5 seconds, and `missed` also counts items dropped on
  their way to a read, as when that loop is closed (#1858).
