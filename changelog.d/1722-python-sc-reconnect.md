---
section: Added
---
- Python SC clients accept frozen ScReconnect settings and reuse their TLS
  policy and identity after connection loss. Client close drains active
  operations and retains joined shutdown when its waiter is cancelled (#1722).
