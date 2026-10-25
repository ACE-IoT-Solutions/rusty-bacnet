---
section: Changed
---
- The TUI keeps one confirmed request outstanding per device across the
  browser, the watch list, writes and COV renewals, and renews two request
  budgets early to cover the wait; managed COV subscriptions take an
  optional renewal gate for this (#1791).
