---
section: Migration notes
---
- **ServerPolicy (Rust API, #2032):** `authorize_mutation` and the
  DeviceCommunicationControl, LifeSafetyOperation and AuditNotification
  methods run on a Tokio blocking-pool thread, so thread-locals set on the
  request's thread don't carry over. A WritePropertyMultiple policy sees the
  database as it was before the request, not after its earlier elements. A
  policy that never returns keeps its thread after `stop()` and keeps the
  process from exiting.
