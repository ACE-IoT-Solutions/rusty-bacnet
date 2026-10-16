---
section: Added
---
- Rust and Python can inspect bounded local SC PEM inputs offline, reporting
  certificate fingerprints, dates, SAN/KU/EKU findings, key consistency and
  advisory Unix key-file mode bits. Existing SC adapters share PEM decoding
  while retaining TLS policy and startup error ordering (#968).
