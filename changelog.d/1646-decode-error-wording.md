---
section: Changed
---
- Decoding messages that change wording (#1646): invalid UTF-8 names how many
  octets were valid, and the application readers' wrong-length Date, Time, REAL and
  BACnetObjectIdentifier gain an `application` prefix. `decode_app_primitive`
  takes only static labels; file records still name their index.
