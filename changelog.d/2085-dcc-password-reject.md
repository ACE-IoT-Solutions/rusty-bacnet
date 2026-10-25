---
section: Fixed
---
- **Wire:** a server rejects a DCC or ReinitializeDevice password over 20
  characters with PARAMETER_OUT_OF_RANGE. DCC used to answer with an Error
  (SERVICES, OTHER) and ReinitializeDevice with INVALID_DATA_ENCODING (#2085).
