---
section: Migration notes
---
- **BBMD helpers (Rust API, #1841):** match `BvlcExchangeError::Refused(ExchangeRefusal::..)` (re-exported from `bacnet_transport::bip`) instead of the text of a `Transport(Error::Encoding(..))` for a request refused before sending.
