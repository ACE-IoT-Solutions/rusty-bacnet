# Upstream reconciliation evidence ledger

Status: Phase 0 baseline frozen on 2026-09-11  
Integration branch: `reconcile/upstream-v0.11-parity`

This ledger is the loss-prevention baseline for
`docs/plans/upstream-reconciliation-workplan.md`. It is planning evidence, not
a conformance claim. Product behavior still has to be ported and rerun on the
integration branch.

## Frozen inputs

| Input | Permanent ref or object | Resolved commit |
|---|---|---|
| Local pre-reconciliation tip | `archive/dev-pre-upstream-reconciliation-20260911` | `bf6922d5efd92da3890c4ede2fbf76dee8c8abb7` |
| Pinned upstream target | `archive/upstream-dev-pinned-20260911` | `a62821b5f961281663b373b901135a8cd8d4ff08` |
| Merge base | `archive/upstream-merge-base-20260911` | `6b9ac4f0f2dfadbf6dbbda88c46ad6a0b0aa43d2` |

At capture time, `reconcile/upstream-v0.11-parity` pointed to the pinned
upstream target and tracked `upstream/dev`. No branch was deleted or
force-pushed. The March-era branches below are **frozen provenance only** and
must not be cherry-picked into this line:

| Frozen branch | Tip |
|---|---|
| `develop` | `cdfa73b80738c0b9e0b14af0a575e636e077b5de` |
| `feature/bvll-router-discovery` | `00c21d6b16acf98a8879e9d4c05acec97e43a567` |
| `feature/routed-device-addressing` | `58c3cfa6982ac57ab47b5a862509380c4c5dab41` |
| `feature/streaming-iam-api` | `62add448395228038ec994ec3adc1d6f4f850374` |
| `fix/cross-subnet-tsm-diagnostics` | `33af6edcda844a3e344281b75c210c52fe077dcb` |
| `fix/cross-subnet-tsm-fallback` | `5c4e3b63b9c1745496f03d4fe78fbf360590d282` |

Remotes at capture time were `origin =
git@github.com:ACE-IoT-Solutions/rusty-bacnet.git` and `upstream =
https://github.com/jscott3201/rusty-bacnet.git` for both fetch and push. Tags
ran from `v0.1.1` through `v0.11.0`; the reconciliation-relevant tag was
`v0.11.0` at `be0df67843e9588e6e3d9b705c02781ec132e1b8`.

The worktree was intentionally not clean because integration work was already
in progress: `.gitignore`, `AGENTS.md`, `Cargo.toml`, client property/target
sources, and Python binding target/read-write sources were modified, while
`docs/plans/` was untracked. These were concurrent changes, not inputs to the
pinned manifests, which read Git objects directly.

Toolchain capture:

- `rustc 1.96.0 (ac68faa20 2026-05-25) (Homebrew)`
- `cargo 1.96.0 (30a34c682 2026-05-25) (Homebrew)`
- `Python 3.14.6`
- `git version 2.50.1 (Apple Git-155)`
- `maturin`: not installed on the capture shell (`command not found`)

All tag tips are reproducible with the refs command in the command record;
recording the relevant release tag above avoids treating unrelated historical
tags as reconciliation inputs.

## Reproducible manifests

Run:

```sh
python3 docs/plans/evidence/upstream-reconciliation/generate_manifests.py
```

The generator emits sorted UTF-8 files under
`docs/plans/evidence/upstream-reconciliation/{local,upstream}/` plus the path
classification. Counts exclude each manifest's header.

| Inventory | Local | Upstream |
|---|---:|---:|
| Workspace membership/default membership/Cargo features | 41 | 40 |
| Rust public exports and methods | 2,551 | 3,517 |
| Python classes/functions/methods/signatures/exceptions | 571 | 385 |
| Test-like Rust/Python files | 121 | 199 |
| Rust/Python test names | 860 | 1,395 |
| Conformance row IDs | 35 | 68 |
| Public-support statement records | 70 | 106 |

The branch-level sanity counts are 20 local commits (19 non-merge), 352
upstream commits, 276 locally changed paths (`+53,330/-6,901`), and 948
upstream changed paths (`+194,496/-17,171`) after the merge base.

`local-only-paths.tsv` defines a local-only path precisely as a path changed in
`BASE..LOCAL` but not changed in `BASE..UPSTREAM`. It classifies all 187 such
paths: 168 `port`, 12 `needs-design`, and 7 `evidence-only`. No local-only path
was classified `obsolete`; obsolescence is at the old implementation/commit
level and must not be inferred merely because a path is absent upstream.
`replace-with-upstream` remains an allowed classification and is used whenever
a future generated dependency artifact qualifies; no path in this exact set
did. The 89 paths changed on both branches require tranche-specific three-way
review and are represented by the commit ledger below rather than mislabeled
as local-only.

### Inventory method and limitations

- Workspace members/default members and feature tables are parsed with
  `tomllib` from every tracked `Cargo.toml`.
- Rust exports are a lexical inventory of line-local `pub` declarations.
  Methods are qualified with the nearest simple `impl` owner. Macro-generated
  exports, multiline declarations, re-export expansion, visibility through
  private modules, and complex impl owners are not semantically resolved.
- Python APIs are parsed from non-test `.py`/`.pyi` ASTs. Method signatures are
  normalized by `ast.unparse`; exception rows also include Rust
  `create_exception!` declarations. Dynamic registrations and PyO3 names not
  represented in stubs or that macro are not inferred.
- Rust test names are functions following a line-local `#[test]` or
  `#[tokio::test]`; Python names are AST functions beginning with `test`.
  Macro-generated/parameterized cases count once and doctests are not counted.
- Conformance row IDs/statuses come from
  `docs/conformance/bacnet-135-2020.json`. Public statements include each row's
  `public_claims` plus lines in the root README and support summary containing
  a support/implementation/conformance/BIBB/PICS signal. This is a deliberately
  broad claim-review queue, not proof that each line makes a support claim.
- Line numbers are branch-local coordinates. Compare semantic columns as well
  as raw rows when code movement changes line numbers.

## Local commit and workstream disposition

Every local non-merge commit has one planned disposition. `Destination` names
the work-plan phase/tranche, not an already-created pull request. `Tests/evidence`
is the minimum evidence family to transplant or rerun.

| Local commit / workstream | Upstream substitute or seam | Destination | Tests/evidence | Disposition |
|---|---|---|---|---|
| `2e9a441` initial ACE Edge Python parity APIs | 0.11 Python surface and endpoint APIs | Phases 2-4 | installed API union; direct/routed RP/RPM/WP/WPM, discovery, BDT/FDT, COV | `needs-design` |
| `f414126` W11 same-port interface isolation | current B/IP ingress/socket implementation | Phase 7 | multi-interface same-port isolation and descriptor cleanup | `needs-design` |
| `7313237` ACE Edge P0 parity documentation | current conformance/API docs | Phases 2 and 11 | claim-by-claim doc and stub audit | `evidence-only` |
| `ef76d0a` Rust-owned runtime R0-R6/G0-G8 | `bacnet-endpoint-core` transaction seam plus current transports | Phase 5 | seven runtime groups; reconcile/rollback/shutdown/resource baselines | `port` |
| `81464c4` maturin-only extension-module | upstream dependency/workspace versions | Phase 2 | workspace check outside Python; clean wheel import | `port` |
| `6b9e154` modular B/IP W1/W2/W3/W11 substrate | upstream B/IP admission, ingress, and fanout core | Phases 6-7 | upstream B/IP limits plus provenance/foreign-device/socket tests | `needs-design` |
| `f8bb3ac` W1 streaming I-Am provenance | upstream merged discovery/event flow | Phase 6 | duplicate-preserving bounded stream and complete BVLL/NPDU metadata | `port` |
| `555d50e` W2 foreign-device exposure | upstream BVLL admission and foreign-device primitives | Phase 6 | status, renewal, recovery, unregister, shutdown | `port` |
| `abf0c64` W3 BBMD control/policy hook | upstream fail-closed bounded BBMD | Phase 7 | management auth/limits/dedup/fanout plus control/snapshot/policy API | `needs-design` |
| `a5e68e6` W4 identity/live NetworkPort | upstream object model and server | Phase 9 | identity validation and live BDT/FDT/FD status properties | `port` |
| `e1b487f` W5 virtual networks/router | upstream NPDU/router handling | Phase 8 | router composition, loop bounds, route expiry, Python counters | `port` |
| `7a040ef` W6 COV criteria/object metadata | upstream COV quotas, scheduler, workers | Phase 9 | criteria matrix, metadata, atomic changes, worker/resource limits | `needs-design` |
| `92dffa7` W7 passive COV/runtime events | upstream unsolicited ACK and routed ACK behavior | Phases 4-5 | direct/routed confirmed/unconfirmed delivery, lag and shutdown | `needs-design` |
| `eea661d` W8 client/APDU error taxonomy | upstream transaction correlation, TSM, segmentation | Phase 3 | stable Python errors and wrong-peer/stale/ambiguous response negatives | `needs-design` |
| `f24df8a` W9 raw decode/tag helpers | upstream encoding/value types | Phase 10 | malformed depth/length/tag limits and proprietary round trips | `port` |
| `6eb8282` W10 APDU observer | upstream ingress/egress seams | Phase 10 | opt-in bounded redacted observer, lag and shutdown | `port` |
| `f0a2564` W12 campus fixture | upstream routing/BBMD core | Phase 11 | clean-wheel isolated campus discovery and routed services | `port` |
| `50b7414` W13 four-way fixture | upstream combined stack | Phase 11 | four-way cross-stack installed suite | `port` |
| `bf6922d` W1-W13 Python binding/evidence rollup | union of all current upstream Python APIs | Phases 2-11 | installed union manifest and each W1-W13 acceptance family | `needs-design` |

The legacy invoke-ID-only cross-subnet fallback represented by the frozen March
branches is `obsolete` and explicitly excluded. It is replaced by upstream's
canonical transaction-peer correlation; no reconciliation tranche may restore
that implementation.

## Command record

Commands were run from the repository root. They are recorded verbatim except
that output redirection was omitted where this document summarizes output.

```sh
git status --short --branch
git remote -v
git branch --format='%(refname:short)\t%(objectname)' | sort
git tag --format='%(refname:short)\t%(objectname)' | sort
git show-ref --verify refs/heads/archive/dev-pre-upstream-reconciliation-20260911
git show-ref --verify refs/heads/archive/upstream-dev-pinned-20260911
git rev-parse 6b9ac4f
git merge-base archive/dev-pre-upstream-reconciliation-20260911 archive/upstream-dev-pinned-20260911
git rev-list --count 6b9ac4f..archive/dev-pre-upstream-reconciliation-20260911
git rev-list --no-merges --count 6b9ac4f..archive/dev-pre-upstream-reconciliation-20260911
git rev-list --count 6b9ac4f..archive/upstream-dev-pinned-20260911
git diff --shortstat 6b9ac4f..archive/dev-pre-upstream-reconciliation-20260911
git diff --shortstat 6b9ac4f..archive/upstream-dev-pinned-20260911
git log --no-merges --format='%H%x09%s' 6b9ac4f..archive/dev-pre-upstream-reconciliation-20260911
rustc --version
cargo --version
python3 --version
git --version
maturin --version
python3 docs/plans/evidence/upstream-reconciliation/generate_manifests.py
find docs/plans/evidence/upstream-reconciliation -type f -print0 | sort -z | xargs -0 shasum -a 256
git diff --check -- docs/plans/evidence/upstream-reconciliation-ledger.md docs/plans/evidence/upstream-reconciliation
```

The two `archive/*` refs pre-existed this evidence generation and were verified
before reading them. Creating or moving refs, deleting branches, and pushing
were intentionally outside the generator.

## Pinned upstream baseline verification

The exact upstream target was checked in an isolated detached worktree before
local product code was added. The temporary worktree was clean and removed
afterward.

| Command | Result | Duration |
|---|---:|---:|
| `cargo fmt --all -- --check` | pass | 1.03 s |
| `cargo check --workspace --locked` | pass | 19.02 s |
| `cargo test --workspace --exclude rusty-bacnet --locked --no-fail-fast` | pass | 101.92 s |
| `cargo check -p rusty-bacnet --tests --locked` | pass | 6.69 s |

The host used Rust/Cargo 1.96.0 and Python 3.14.6, while upstream pins Rust
1.97.1 and advertises Python 3.11-3.13 for wheels. Two environment-dependent
Rust integration tests and one doctest were ignored. Clippy, audit, deny,
MSRV, the full feature matrix, and cross-platform jobs remain open gates; the
table records a locally applicable baseline, not complete G1.

The exact pinned target was subsequently rebuilt in another isolated detached
worktree using `uvx maturin`, a fresh uv-managed CPython 3.13.14 environment,
and the unmodified upstream `pyproject.toml`. The release wheel
`rusty_bacnet-0.11.0-cp313-cp313-macosx_11_0_arm64.whl` built successfully,
installed into the empty environment, and imported from `site-packages` rather
than the source tree. The complete upstream Python suite passed: 91 tests and
1,268 subtests in 44.96 seconds. The deterministic upstream API manifest is
retained under `upstream/python-api.tsv` in this evidence directory.

## Reconciled implementation record

The Phase 0 table above records the decision made before implementation. The
following table records the resulting upstream-first destinations; none of the
19 local commits was replayed wholesale.

| Workstream | Resulting commits / retained upstream owner | Final disposition |
|---|---|---|
| Python contract and packaging | `5986388`, `1691d8c`, `2405469`, plus compound-value fixes `3d6098e`, `6b162a2`, `a54cbd3` | ported as a reviewed API union |
| Typed direct/routed operations and errors | `3899a5e`, `e371333`, `64825e4`; upstream endpoint coordinator, routed-path limits, TSM, and segmentation retained | ported extension over upstream ownership |
| Aggregate runtime | `8c2f076`, `00d6917`, `5207d89`, `b206ffd`, `ddb248a` | ported without a second transaction coordinator |
| I-Am and foreign-device lifecycle | `cdb3db4`, `b206ffd`, `e6e2c66`, `782626a`, `0c63701` | ported over bounded upstream B/IP admission |
| Socket isolation, BBMD control, and BVLL policy | `f4c72e7`, `cdb3db4`, `5207d89`, `4bf901b`, `1663788` | ported; policy is narrowing-only |
| Virtual transport and composable router | `a5470e5`, `3dde5e5`, `f5c08de` | ported as a bounded simulation adapter |
| Device, NetworkPort, COV metadata, and PICS | `9dc89c2`, `0462dd1`, `9898915`, `6f968b5`, `07534c7`, `e6866f8` | ported onto upstream object/COV ownership |
| Raw decode, APDU observation, and passive COV | `97018ed`, `67d84ea`, `6cf0023`, `5207d89`, `ddb248a` | ported as passive bounded observation |
| W1-W13 evidence | `8d749db`, `782626a`, `a5c72ac`, `232b76c`, `38bcdb4`, `aade25c`, `4bf901b`, `54030c5`, `725733b`, `e72828c`, `0c63701` | replaced by current clean-wheel fixtures |
| Legacy invoke-ID-only routed fallback | upstream canonical peer and generation-qualified coordinator | obsolete; intentionally not restored |

`git diff --diff-filter=D --name-only a62821b5..HEAD` returned no paths during
the completion audit. Thus no file, workspace member, or test family from the
pinned upstream tree was removed. The installed API union report contained
1,283 rows: 1,233 on the integration branch, 1,095 on old local, and 787 on
upstream. Twelve rows were superseded, 38 removals were intentional and
reviewed, and zero removals were unexplained.

## Reconciled acceptance evidence

| Boundary | Result | Evidence |
|---|---|---|
| Installed Python | CPython 3.13.14 release wheel; 145 tests in 46.725 s, all pass; import performed from `/tmp` outside the checkout | `target/upstream-reconciliation/python-suite-0c63701` |
| W2 foreign-device lifecycle | Pending, Registered, TTL/2 renewal, broadcast delivery, BBMD service stop, Expired, fresh service reconstruction, recovery, rejection `0x0030`, second expiry/recovery, zero FDT, and clean shutdown | `target/upstream-reconciliation/w2-link-restart-fix` |
| W13 and component fixtures | all four bacpypes3/rusty client/server legs pass; W1, W3, W5, W6, and W12 modules, semantic comparison, and cleanup pass | `target/upstream-reconciliation/w13-e72828c`; result JSON SHA-256 `f97c56765addb33b66e8e91a8b0514d24715ade40922ea960ed0384a44b363a1` |
| Server segmentation configuration | all four modes agree across server behavior, I-Am, and Device properties; `NONE` omits conditional fields; W13 exercises segmented Object_List | `e72828c`; `test_server_segmentation_config.py`; W13 artifact above |

The W13 source image was built from one clean exact source archive. The first
invocation used a relative artifact path that Podman Compose interpreted as a
named volume and failed before protocol work; the retained result above is the
complete rerun using an absolute bind path and freshly built images.

## Environment coverage and explicit skips

| Environment | Disposition |
|---|---|
| macOS arm64, CPython 3.13.14 | clean wheel and complete installed suite passed |
| Linux arm64, CPython 3.11 container | W2/W13 and component fixtures passed from built wheels; final SC fixture recorded separately below |
| Rust 1.97.1 Linux/macOS and MSRV 1.93 | final command results are recorded in the release-gate section below |
| Windows | unavailable in this workspace; environmental skip, not a pass |
| CPython 3.11/3.12 complete installed suites | not separately run; Linux fixture coverage is narrower than the complete CPython 3.13 suite |
| Physical MS/TP hardware | unavailable; unit/simulated transport evidence only |
| Full legacy Linux/macOS same-port/wildcard/subnet-broadcast matrix | not reproduced; current focused socket tests plus Linux W3 cover the supported opt-in boundary |

## Retained limitations and cutover

- The full client/server TSM transition audit and configurable server segment
  retry behavior remain open. Python exposes a consistent segmentation mode and
  the current APDU segment timeout; this does not promote the conformance row.
- Virtual transport is an in-process simulation and contributes no Standard
  135 data-link or PICS claim. Await `stop()`: dropping a running server can
  retain detached task/membership ownership. Generic server restart or a retry
  after registration draining does not guarantee registered-object retention.
- BVLL callbacks can only narrow native decisions. Native validation,
  management, foreign-device, fanout, rate, capacity, and amplification denial
  always wins.
- Runtime alias selection is limited to currently observed
  attachment-qualified paths; it is not durable global identity resolution.
- The rollback point is
  `archive/dev-pre-upstream-reconciliation-20260911` at `bf6922d5`. Before
  advancing `dev`, push/verify that permanent ref, tag the reconciled tip, run
  G0-G7 on that immutable tip, then fast-forward `dev`. Roll back by restoring
  `dev` to the archived ref through the repository's protected-branch process;
  do not delete either provenance ref until post-cutover validation finishes.
