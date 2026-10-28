---
section: Migration notes
---
- **Python router:** Health now reports only upstream-native state, and SC
  router receive limits are fixed at 1497 octets NPDU and 5705 octets BVLC;
  other limits and `failover_hub` are rejected until the upstream transport
  can preserve initial TLS failover identity. Post-stop health and route data
  are last-known snapshots rather than live telemetry.
