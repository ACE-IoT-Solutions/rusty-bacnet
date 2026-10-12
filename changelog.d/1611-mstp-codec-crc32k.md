---
section: Added
---
- `bacnet-mstp-codec` exports the CRC-32K (`crc32k`,
  `crc32k_accumulate_all`, `CRC32K_RESIDUAL`), the COBS Length bounds
  `MIN_COBS_LENGTH` and `MAX_COBS_LENGTH`, and a `crc32k-table` feature
  that trades about 1 KiB of flash for a faster CRC-32K (#1611).
