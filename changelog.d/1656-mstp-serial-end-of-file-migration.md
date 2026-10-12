---
section: Migration notes
---
- **MS/TP serial ports (Rust API, #1656):** a custom `SerialPort::read` must
  wait until at least one octet has arrived and return `Ok(0)` only at end of
  file, never to mean "nothing yet" (on a timeout, say). The transport now
  takes three `Ok(0)` reads in a row as the port gone and stops, as it does
  at a read error.
