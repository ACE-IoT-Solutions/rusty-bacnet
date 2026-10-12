---
section: Added
---
- `StreamDecoder::with_reserved_cobs` checks the CRC-32K of the reserved
  COBS frame types (34-127) and yields a good one as a frame of unknown
  type, as a receiver with COBS support must; `FrameType::is_extended_data`
  is public (#1611).
