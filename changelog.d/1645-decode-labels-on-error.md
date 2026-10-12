---
section: Changed
---
- **Rust API:** Service decoders no longer format and allocate a context label
  for each member of a decode that succeeds; a label is built only when an
  error names it, with the same text, and `decode_app_primitive` takes its
  label as any `impl Display` rather than a `&str` (#1645).
