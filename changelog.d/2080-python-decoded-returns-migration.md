---
section: Migration notes
---
- **Python decoded returns (#2080):** `create_object` returns an
  `ObjectIdentifier`, not `bytes`. `atomic_read_file` returns a dict; take
  the octets from `["data"]` or `["records"]`. A bit string `.value`, the
  alarm summary's `acknowledged_transitions`, Event_Parameters bit strings,
  and log `status_flags`, `log_status` and `bitstring` data are `BitString`
  objects: read `.unused_bits` and `.data` where you read the dict, tuple or
  int before.
