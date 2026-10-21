#!/usr/bin/env bash
# plan-skip.sh regressions (#1466), in a throwaway repository with a stub for
# gh's check-runs call. No network.
set -euo pipefail

plan="$(cd "$(dirname "$0")" && pwd)/plan-skip.sh"
fixture=$(mktemp -d)
# So the no-repository case can't find an enclosing one.
export GIT_CEILING_DIRECTORIES=$fixture
trap 'rm -rf "$fixture"' EXIT
cd "$fixture"

# The stub prints $STUB_JSON, or fails when it is unset, and logs its
# arguments so the tests can check the query.
cat >stub-gh <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB_LOG"
[ -n "${STUB_JSON:-}" ] || { echo "HTTP 502" >&2; exit 1; }
printf '%s\n' "$STUB_JSON"
EOF
chmod +x stub-gh
export PLAN_SKIP_GH=$fixture/stub-gh STUB_LOG=$fixture/gh.log
export GITHUB_EVENT_NAME=push GITHUB_REF=refs/heads/dev GITHUB_REPOSITORY=owner/repo
export GITHUB_OUTPUT=$fixture/output GITHUB_STEP_SUMMARY=$fixture/summary

green='{"check_runs":[{"id":1,"started_at":"2026-10-20T10:00:00Z","conclusion":"success","app":{"slug":"github-actions"}}]}'
red='{"check_runs":[{"id":1,"started_at":"2026-10-20T10:00:00Z","conclusion":"failure","app":{"slug":"github-actions"}}]}'
none='{"check_runs":[]}'
# A failed attempt, then a green re-run.
rerun='{"check_runs":[{"id":2,"started_at":"2026-10-20T11:00:00Z","conclusion":"success","app":{"slug":"github-actions"}},{"id":1,"started_at":"2026-10-20T10:00:00Z","conclusion":"failure","app":{"slug":"github-actions"}}]}'
# A green run, then a re-run still in progress.
pending='{"check_runs":[{"id":1,"started_at":"2026-10-20T10:00:00Z","conclusion":"success","app":{"slug":"github-actions"}},{"id":2,"started_at":"2026-10-20T11:00:00Z","conclusion":null,"app":{"slug":"github-actions"}}]}'
# Another app's check run with the same name.
other_app='{"check_runs":[{"id":1,"started_at":"2026-10-20T10:00:00Z","conclusion":"success","app":{"slug":"someone-else"}}]}'

fail() {
  echo "FAIL: plan-skip regression: $1" >&2
  [ -f result.log ] && cat result.log >&2
  exit 1
}

# expect <true|false> <case> [check name]: runs plan-skip.sh in repo/ and
# checks the one skip= line it wrote and that it exited 0.
expect() {
  : >"$GITHUB_OUTPUT"
  : >"$GITHUB_STEP_SUMMARY"
  : >"$STUB_LOG"
  (cd repo && bash "$plan" "${3:-CI OK}") >result.log 2>&1 || fail "$2: exited non-zero"
  [ "$(cat "$GITHUB_OUTPUT")" = "skip=$1" ] || fail "$2: expected skip=$1, wrote '$(cat "$GITHUB_OUTPUT")'"
  grep -q "^skip=$1: " result.log || fail "$2: no log line"
  grep -q '^### Plan: ' "$GITHUB_STEP_SUMMARY" || fail "$2: no summary"
  echo "ok: $2 -> skip=$1"
}

# A dev history with a crate, then a PR branch off it.
reset_repo() {
  rm -rf repo
  git init -q -b dev repo
  (
    cd repo
    git config user.email ci@example.invalid
    git config user.name CI
    mkdir -p crates/a/src scripts/ci .github/workflows
    echo 'fn main() {}' >crates/a/src/main.rs
    echo '[package]' >crates/a/Cargo.toml
    echo 'version = 3' >Cargo.lock
    echo 'features=a' >scripts/ci/local-macos.sh
    echo 'name: CI' >.github/workflows/ci.yml
    git add -A && git commit -q -m base
    git checkout -q -b pr
  )
}

# pr_commit <path> <line>: appends to a file on the PR branch.
pr_commit() {
  (cd repo && echo "$2" >>"$1" && git add -A && git commit -q -m "pr: $1")
}

# merge_pr: the lead's --no-ff merge of the PR into dev.
merge_pr() {
  (cd repo && git checkout -q dev && git merge -q --no-ff --no-edit pr)
}

# A PR that changes only code, updated to dev's tip, then merged: the merge
# tree is the PR head's.
reset_repo
pr_commit crates/a/src/main.rs '// change'
merge_pr
export STUB_JSON=$green
expect true 'tree-equal merge, green CI OK on HEAD^2'
pr_head=$(cd repo && git rev-parse 'HEAD^2')
grep -qF "api repos/owner/repo/commits/$pr_head/check-runs?check_name=CI%20OK&filter=all&per_page=100" "$STUB_LOG" \
  || fail 'query names HEAD^2 and the encoded check name'
expect true 'Native OK is the check native-tests.yml names' 'Native OK'
grep -qF 'check_name=Native%20OK' "$STUB_LOG" || fail 'Native OK query'
grep -qF 'Plan: skip the heavy jobs' "$GITHUB_STEP_SUMMARY" || fail 'skip summary'

STUB_JSON=$rerun expect true 'a failed attempt, then a green re-run'
STUB_JSON=$red expect false 'red aggregate on HEAD^2'
grep -qF 'CI OK on the PR head' result.log || fail 'red reason names the check'
grep -qF 'is failure' result.log || fail 'red reason'
STUB_JSON=$none expect false 'missing aggregate on HEAD^2'
grep -qF 'is missing' result.log || fail 'missing reason'
STUB_JSON=$pending expect false 'newest aggregate run still in progress'
STUB_JSON=$other_app expect false 'same-named check from another app'
grep -qF 'is missing' result.log || fail 'other-app reason'
STUB_JSON='not json' expect false 'unparseable check-runs response'
STUB_JSON='' expect false 'check-runs API call fails'
grep -qF 'Could not read the check runs' result.log || fail 'API failure reason'
grep -qF 'Plan: run every job' "$GITHUB_STEP_SUMMARY" || fail 'run summary'

GITHUB_EVENT_NAME=pull_request expect false 'pull_request event'
GITHUB_EVENT_NAME=schedule expect false 'scheduled run'
GITHUB_EVENT_NAME=workflow_dispatch expect false 'manual run'
GITHUB_REF=refs/heads/main expect false 'push to main'
GITHUB_REF=refs/tags/v1.0.0 expect false 'tag push'
[ ! -s "$STUB_LOG" ] || fail 'non-dev events must not call the API'

# A behind merge: dev moved after the PR branched, so the merge tree is new.
reset_repo
pr_commit crates/a/src/main.rs '// change'
(cd repo && git checkout -q dev && echo '// dev moved' >>crates/a/src/lib.rs && git add -A && git commit -q -m 'dev moves')
merge_pr
expect false 'behind merge (trees differ)'
grep -qF 'different tree' result.log || fail 'behind reason'
[ ! -s "$STUB_LOG" ] || fail 'behind merge must not call the API'

# A plain commit pushed to dev.
reset_repo
(cd repo && git checkout -q dev && echo '// direct' >>crates/a/src/main.rs && git commit -q -am direct)
expect false 'non-merge push'
grep -qF 'is not a merge commit' result.log || fail 'non-merge reason'

# Tree-equal merges whose PR changed a cache-key input.
for input in Cargo.lock crates/a/Cargo.toml rust-toolchain.toml .cargo/config.toml \
  .github/workflows/ci.yml .github/ci-pins.env scripts/ci/local-macos.sh; do
  reset_repo
  (cd repo && mkdir -p "$(dirname "$input")")
  pr_commit "$input" 'changed = true'
  merge_pr
  expect false "cache-key input $input changed"
  grep -qF "$input" result.log || fail "reason names $input"
done

# A file outside the cache inputs, beside them, still skips.
reset_repo
pr_commit scripts/ci/other.sh 'echo'
merge_pr
expect true 'non-key script change, tree-equal merge'

# Outside a git repository: fails open.
rm -rf repo && mkdir repo
expect false 'no git repository'

echo "plan-skip regressions passed"
