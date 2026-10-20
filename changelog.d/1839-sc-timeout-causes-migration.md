---
section: Migration notes
---
- **SC connect causes (Rust and Python API, #1839):** `ScDisconnectCause::ConnectTimeout`
  is gone: match `DialTimeout { stage }`, `ConnectAcceptTimeout` and the new
  `UpgradeRefused { http_status }`. A dial timeout is now
  `ScConnectError::DialTimeout` in `Error::Transport`, not `Error::Timeout`;
  `ScConnectError::WebSocket` gains `http_status`. In Python, match
  `"dial_timeout"` or `"connect_accept_timeout"` instead of `"connect_timeout"`.
