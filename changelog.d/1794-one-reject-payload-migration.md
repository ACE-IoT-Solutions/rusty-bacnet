---
section: Migration notes
---
- **Reject payload (Rust API, #1794):** `RejectMessageToNetwork` and
  `decode_reject_message_to_network` leave `bacnet_encoding::npdu`, and
  router-core's `message::Reject` is gone: use
  `bacnet_npdu::RejectMessageToNetwork`, whose `decode` (was `parse`) returns
  a `PayloadLengthError` and whose `dnet` field is now `network`.
