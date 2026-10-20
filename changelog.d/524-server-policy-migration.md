---
section: Migration notes
---
- **Server policy (Rust API, #524):** the four `*_authorizer` fields and setters,
  and the `LifeSafetyOperationAuthorizer`, `AuditNotificationAuthorizer` and
  `UnconfirmedAuditNotificationAuthorizer` aliases, are gone: implement
  `ServerPolicy` or chain `FnPolicy` closures. `.policy(..)` replaces any earlier
  policy, so merge them into one. `MutationAuthorizer` is now
  `mutation::DeviceWriteAuthorizer`. See
  [Server authorization policy](docs/mutation-policy.md).
