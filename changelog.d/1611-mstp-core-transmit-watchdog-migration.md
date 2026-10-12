---
section: Migration notes
---
- **MS/TP transmit watchdog (Rust API, #1611):** a `transmit_watchdog` set
  by hand must exceed `MstpCoreConfig::minimum_transmit_watchdog()`, the
  longest frame's time plus T_postdrive and 1 ms, or validation returns
  `TransmitWatchdogTooShort`. Only rates below about 5.1 kbaud, or
  extended frames below about 15.2 kbaud, are affected at one second.
