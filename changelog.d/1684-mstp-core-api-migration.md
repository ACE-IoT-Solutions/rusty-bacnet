---
section: Migration notes
---
- **MS/TP core (Rust API, #1684):** renamed: `QueueRejectReason` to
  `QueueRefusal`, `max_npdu_length` to `max_npdu`; `NetworkPriority`
  converts with `try_from` and `u8::from`. Gone: `ReplyDecision::Cancelled`
  (use `Postponed`), the `raw` ids (use `generation`), `Starved`,
  `drops_starved`, `BROADCAST_DESTINATION` (use the codec's `BROADCAST_MAC`)
  and the queues' `with_starvation_limit` (set the config's). Errors need `_`
  ([all of it](docs/rust-api.md#bacnet-mstp-core-firmware)).
