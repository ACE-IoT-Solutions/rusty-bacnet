---
section: Migration notes
---
- Update exhaustive Rust matches on `ConfirmedCOVNotificationResponse` to
  handle `Error(class, code)`. Python callers can enable strict admission with
  `confirmed_cov_subscription_limit`; the default still acknowledges every
  decoded confirmed COV notification.
