---
section: Migration notes
---
- **Breaking (Rust API, #1716):** Router-core adapters must retain each send's
  `reject_origin` and pass it to `refused_later`; drain the resulting actions.
  Custom transports report a definite length refusal with `Error::SendTooLong`
  only before queueing or writing any bytes. Other failures keep their existing
  error variants.
