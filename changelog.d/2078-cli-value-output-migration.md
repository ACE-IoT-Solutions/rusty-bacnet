---
section: Migration notes
---
- **CLI value output (#2078):** scripts that matched `enumerated(N)`,
  `bits(...)`, `ANALOG_INPUT:1` inside a value, or `[raw: ...]` in the
  `value` text should read the new `decoded` field instead (shapes in
  [the CLI reference](docs/CLI.md#value-output)); `value` stays a string,
  and undecoded octets now print as `[raw 0e 0f]`.
