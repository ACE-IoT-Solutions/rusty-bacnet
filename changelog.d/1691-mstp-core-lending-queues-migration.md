---
section: Migration notes
---
- **MS/TP queues (Rust API, #1691):** call `QueueStorage::dequeue` or
  `dequeue_kind` with the longest NPDU you take, in place of `dequeue_into`
  and `dequeue_kind_into` with a scratch, and read the octets from
  `DequeuedNpdu::npdu`, which replaces `len`. An implementation lends them
  from its own storage. Queue slots hold at most 65,535 octets. Drop
  `heapless` from `bacnet-mstp-core`'s features: `HeaplessQueues` needs none.
