---
section: Migration notes
---
- **Typed errors and units (Rust API, #915):** `Error::Protocol`,
  `Error::Structured` and `TsmResponse::Error` hold `ErrorClass`/`ErrorCode`,
  and `Error::protocol` takes them; `Reject`/`Abort` hold `RejectReason`/
  `AbortReason`. Match on the constants, or use `from_raw`/`to_raw`. Pass
  `EngineeringUnits` to the analog, accumulator, pulse converter and loop
  constructors and to `StagingConfig::units`.
