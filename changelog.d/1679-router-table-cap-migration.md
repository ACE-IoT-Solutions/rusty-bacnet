---
section: Migration notes
---
- **Router table (Rust API, #1679):** A local `table().add_learned(..)` or
  `add_direct(..)` returns `bool` and refuses a new network once the table
  holds 256 routes (`ROUTE_CAP`), where local adds had no limit before;
  check the result. Don't call `table()` again while its guard lives on the
  same thread: the lock isn't reentrant.
