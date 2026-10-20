---
section: Migration notes
---
- **Server policy (Rust API, #524):** replace `mutation_authorizer`,
  `life_safety_operation_authorizer`, `audit_notification_authorizer` and
  `unconfirmed_audit_notification_authorizer` (fields and builder setters) with
  one `.policy(..)`: implement `ServerPolicy`, or chain closures on
  `FnPolicy::new()`. The endpoint's `MutationAuthorizer` type is now
  `mutation::DeviceWriteAuthorizer`. See
  [Server authorization policy](docs/mutation-policy.md).
