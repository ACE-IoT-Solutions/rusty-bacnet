---
section: Fixed
---
- Client stress benchmarks now report typed request failures and invalidate incomplete runs instead of silently omitting failed clients or steps. Repeated comparisons retain reproducible inputs and separate successful latency from failure timing (#1979).
