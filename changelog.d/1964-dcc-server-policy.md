---
section: Security
---
- `ServerPolicy` can now decide DeviceCommunicationControl by the requester's
  verified certificate principal, asked after the password and source checks
  and before the disable rate limit is charged. The default still allows, and a
  panic denies (#1964).
