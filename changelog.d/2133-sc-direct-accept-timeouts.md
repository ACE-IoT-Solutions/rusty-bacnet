---
section: Fixed
---
- `DirectListener::start` rejects an idle timeout of zero or over a day, and a
  connect timeout of zero or over 300 seconds, instead of closing every link,
  refusing every peer or panicking (#2133).
