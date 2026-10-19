---
section: Changed
---
- **Python API:** an SC hub host that does not resolve now reads "WebSocket
  host lookup for HOST failed" rather than a TCP dial failure, in `--sc`
  output and `BacnetTransportError`; a name with no addresses gets errno
  ENOENT rather than EINVAL (#967).
