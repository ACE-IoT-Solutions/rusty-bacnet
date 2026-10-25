---
section: Migration notes
---
- **CLI value output (#2078):** scripts that matched `enumerated(N)`,
  `bits(...)`, `ANALOG_INPUT:1` inside a value, or `[raw: ...]` in the
  `value` text should read the new `decoded` field instead, whose shapes are
  listed in [the CLI reference](docs/CLI.md#value-output); `value` stays a
  string.
