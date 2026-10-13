---
section: Added
---
- New `bacnet-mstp-codec` crate: an allocation-free `no_std` codec for
  standard and extended MS/TP frames, with a stream decoder; `alloc`,
  `bytes` and `heapless` add sinks, `crc32k-table` a faster CRC-32K. The
  MS/TP transport re-exports it (`mstp::codec`). By Andrew Rodgers, ACE IoT
  Solutions (#1604).
