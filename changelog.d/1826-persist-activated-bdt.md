---
section: Added
---
- A BDT activated through the Network Port object is saved to the BBMD's
  persisted-BDT file, so a restart keeps it; a failed save is logged and
  counted (`BipTransport::bdt_persist_failures`) and leaves the new BDT in use
  (#1826).
