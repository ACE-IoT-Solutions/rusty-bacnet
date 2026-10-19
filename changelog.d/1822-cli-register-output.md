---
section: Changed
---
- **CLI:** `register` and `unregister` print "… accepted" on success, without
  the result code, in JSON mode too; a NAK fails the command with exit 1
  (#1822).
