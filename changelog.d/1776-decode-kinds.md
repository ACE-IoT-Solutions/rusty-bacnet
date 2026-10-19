---
section: Changed
---
- Two refusals keep the inner fault's kind (#1776): the weekly and exception
  schedule decoders no longer report every entry fault as `InvalidEncoding`,
  and a ReadRange log record's range error converts to `OutOfRange`.
