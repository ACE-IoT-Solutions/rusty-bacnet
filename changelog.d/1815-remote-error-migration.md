---
section: Migration notes
---
- **Remote errors (Rust API, #1815):** `Error::Protocol`, `Error::Reject` and
  `Error::Abort` are now `Error::Remote(RemoteError::Error { class, code })`,
  `RemoteError::Reject { reason }` and `RemoteError::Abort { reason }`; build
  them with `Error::protocol(class, code)`, `Error::reject` and `Error::abort`.
  The old three-argument `Error::protocol` is `Error::with_detail`. The
  server's `Refusal` is `RemoteError`, and `TsmResponse::{Error, Reject,
  Abort}` are `TsmResponse::Remote { error, detail }`.
