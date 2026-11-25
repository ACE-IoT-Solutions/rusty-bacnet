---
section: Migration notes
---
- For exact UCS-4 bytes, read `value.read_context.raw` on RP/RPM results or `row["raw"]` on COV notifications. A decoded scalar string's `.octets` is its canonical UTF-8 write encoding.
