---
section: Migration notes
---
- **Python `BitString` indexing (#2080):** `bits[i]` and iteration give bit
  `i` as a bool. A log `bitstring` datum was an `(unused_bits, bytes)` tuple:
  `datum["bitstring"][0]` is now bit 0, and `unused, data = ...` fails or
  unpacks two bits; use `.unused_bits` and `.data`. `result_flags == (True,
  True, False)` is now False; indexing and unpacking it still work.
