---
section: Security
---
- `ServerPolicy` can now decide DeviceCommunicationControl by the requester's
  verified certificate principal, asked after the password, source and rate
  checks. The default still allows, and a panic denies (#1964).
