---
section: Changed
---
- Encoding messages that change wording (#1802): a malformed ListElement or
  COVNotificationMultiple raw value names its context after the decoder's
  offset. Restated refusals also drop an item index and a range prefix,
  which no caller reaches today.
