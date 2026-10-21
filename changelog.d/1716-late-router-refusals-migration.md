---
section: Migration notes
---
- **Late send refusals (Rust API, #1716):** to return a length rejection for
  a send refused after queueing, a router-core adapter keeps each send's
  `reject_origin`, passes it to `Router::refused_later` and drains the
  resulting actions. A custom transport returns the new `Error::SendTooLong`
  for a length refusal only before it queues or writes any bytes.
