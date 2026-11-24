---
section: Added
---
- **Breaking (Rust API):** Clients can opt into bounded local registrations for
  confirmed COV. They acknowledge matching single-object notifications and
  return a service Error for unknown subscriptions. Rust response policies
  can now return a service Error.
