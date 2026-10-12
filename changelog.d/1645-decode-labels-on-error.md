---
section: Changed
---
- Service decoders no longer format and allocate a context label for each
  member of a decode that succeeds; a label is built only when an error names
  it, with the same text (#1645).
