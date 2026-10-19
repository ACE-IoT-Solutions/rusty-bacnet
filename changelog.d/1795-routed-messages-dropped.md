---
section: Fixed
---
- **Wire:** the non-router `NetworkLayer` drops a network message whose DNET
  names one network instead of handing it to its control receiver, as it
  already did for APDUs (Clause 6.5.2.2, #1795).
