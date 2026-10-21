---
section: Migration notes
---
- **SC dial timeouts (Rust API, #1839):** a hub dial that times out is now
  `Error::Transport` carrying `ScConnectError::DialTimeout { timeout, stage }`
  (read it with `ScConnectError::from_io_error`), not `Error::Timeout`; a
  missing Connect-Accept is still `Error::Timeout`. Python still raises
  `BacnetTimeoutError` for both.
