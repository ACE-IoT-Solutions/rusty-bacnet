---
section: Added
---
- `bacnet-mstp-codec` computes the CRC-32K (`crc::crc32k`), exports the
  COBS Length bounds `MIN_COBS_LENGTH` and `MAX_COBS_LENGTH`, and has a
  `crc32k-table` feature that trades about 1 KiB of flash for a faster
  CRC-32K (#1611).
