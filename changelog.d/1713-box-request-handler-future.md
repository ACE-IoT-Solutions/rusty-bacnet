---
section: Changed
---
- The server boxes each request handler's future at spawn instead of
  copying the several-kilobyte future many times: one B/IP client on an
  Apple M5 gets about 18% more requests a second on top of the
  handler-reaping change (#1713).
