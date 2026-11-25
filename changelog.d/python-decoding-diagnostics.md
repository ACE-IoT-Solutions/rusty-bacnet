---
section: Added
---
- Python callers can catch `BacnetDecodingError` and inspect the fault kind and byte offset. If a validated ReadProperty response fails value projection, the exception also exposes the responder’s property reference and exact value bytes.
