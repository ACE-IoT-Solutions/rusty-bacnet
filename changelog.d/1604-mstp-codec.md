---
section: Added
---
- New `bacnet-mstp-codec` crate: an allocation-free `no_std` codec for
  standard and extended MS/TP frames and their CRCs, encoding into caller
  storage, with an incremental stream decoder. The MS/TP transport
  re-exports it as `mstp::codec`. Contributed by Andrew Rodgers (ACE IoT
  Solutions) (#1604).
