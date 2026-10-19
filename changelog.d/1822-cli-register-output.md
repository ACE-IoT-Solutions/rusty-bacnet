---
section: Changed
---
- **CLI:** `register` and `unregister` print "… accepted" on success, without
  the result code, in JSON mode too; a NAK fails the command with exit 1. The
  shell's `register` refuses a TTL of 0 and renews no faster than once a
  second (#1822).
