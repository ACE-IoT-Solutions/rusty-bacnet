---
section: Added
---
- CLI `file-read` and `file-write` now reach routed devices using existing network-and-instance or network-and-MAC targets. Every stream or record read window preserves checked acknowledgements and staged output; stream writes retain `--allow-write`. The Rust client also supports routed record writes (#1786).
