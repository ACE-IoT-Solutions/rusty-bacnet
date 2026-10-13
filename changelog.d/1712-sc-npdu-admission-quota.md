---
section: Changed
---
- A BACnet/SC node queues up to 256 received NPDUs, 32 from any one peer (was 64 and 4), so concurrent requests to one device no longer stall until the APDU timeout, and dropped NPDUs log a throttled warning (#1712). For the old guard, call `with_npdu_per_origin_limit(4)`.
