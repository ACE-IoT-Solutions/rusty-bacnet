---
section: Changed
---
- **Wire:** `BACnetClient::who_has` sends its Who-Has as a global broadcast,
  as `who_is` does, so `find` and `:find` reach devices behind routers (#960).
