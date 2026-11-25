---
section: Migration notes
---
- **Breaking (release tooling):** Use `scripts/build-python-release.sh <version>` for local validation. Private publication runs through the tagged GitHub release workflow. The local build helper no longer takes `--build-only` or uploads distributions.
