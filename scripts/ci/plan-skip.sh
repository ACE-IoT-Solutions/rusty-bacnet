#!/usr/bin/env bash
# Decides whether a push to dev can skip the heavy jobs (#1466). A merge made
# with --no-ff from a PR head that already contained dev's tip has exactly
# that head's tree, and the PR's own runs already tested it. It skips only
# when all of these hold, and runs on anything else, errors included:
#
#   - the event is a push to refs/heads/dev;
#   - HEAD is a merge, and its tree equals HEAD^2's (the PR head);
#   - no rust-cache key input changed between HEAD^1 and HEAD, so the dev
#     caches PRs restore stay current without this run saving new ones;
#   - the newest GitHub Actions check run named <check> on HEAD^2 concluded
#     success.
#
# The checkout needs fetch-depth: 2 for both parents. Writes skip=true or
# skip=false to $GITHUB_OUTPUT and the reason to $GITHUB_STEP_SUMMARY, and
# logs it. It exits 0 whatever it decides; a failure to decide means run.
#
#   GH_TOKEN=... GITHUB_EVENT_NAME=push GITHUB_REF=refs/heads/dev \
#     GITHUB_REPOSITORY=owner/repo bash scripts/ci/plan-skip.sh "CI OK"
#
# PLAN_SKIP_GH names a command to run in place of gh (the tests stub it).
# scripts/ci/test-plan-skip.sh covers each case.
set -euo pipefail

check=${1:?usage: plan-skip.sh "<aggregate check name>"}
gh_cmd=${PLAN_SKIP_GH:-gh}

# What feeds the workflows' rust-cache keys: every Cargo.lock and Cargo.toml
# in the repository, the root toolchain and cargo config files, the workflow
# files (the env-vars values such as LINUX_FEATURES, the CARGO_* and RUST*
# variables, the job ids) and the pins beside them, and local-macos.sh, whose
# features= line is native-tests.yml's NATIVE_FEATURES. The runner image's own
# toolchains (native-tests.yml keeps them) also enter the native key; no diff
# can see an image update.
cache_inputs=(
  ':(glob)**/Cargo.lock'
  ':(glob)**/Cargo.toml'
  rust-toolchain
  rust-toolchain.toml
  .cargo/config
  .cargo/config.toml
  .github/
  scripts/ci/local-macos.sh
)

decide() {
  local skip=$1 reason=$2
  echo "skip=$skip" >>"${GITHUB_OUTPUT:-/dev/null}"
  if [ "$skip" = true ]; then
    echo "### Plan: skip the heavy jobs" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  else
    echo "### Plan: run every job" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  fi
  echo "$reason" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  echo "skip=$skip: $reason"
  exit 0
}

event=${GITHUB_EVENT_NAME:-}
ref=${GITHUB_REF:-}
if [ "$event" != push ] || [ "$ref" != refs/heads/dev ]; then
  decide false "Not a push to dev (${event:-no event} on ${ref:-no ref}); only those can skip."
fi
repo=${GITHUB_REPOSITORY:-}
[ -n "$repo" ] || decide false "GITHUB_REPOSITORY is not set."

head=$(git rev-parse HEAD) || decide false "No HEAD commit."
if ! second=$(git rev-parse -q --verify 'HEAD^2^{commit}'); then
  decide false "$head is not a merge commit."
fi
first=$(git rev-parse 'HEAD^1') || decide false "Cannot read HEAD^1 of $head."

head_tree=$(git rev-parse 'HEAD^{tree}')
second_tree=$(git rev-parse "$second^{tree}")
if [ "$head_tree" != "$second_tree" ]; then
  decide false "The merge $head has a different tree from its PR head $second: the PR was behind dev, so this tree is untested."
fi

status=0
git diff --quiet "$first" HEAD -- "${cache_inputs[@]}" || status=$?
if [ "$status" -eq 1 ]; then
  changed=$(git diff --name-only "$first" HEAD -- "${cache_inputs[@]}" | head -n 5 | paste -sd ' ' -)
  decide false "Cache-key inputs changed since $first ($changed), so this run saves new dev caches."
elif [ "$status" -ne 0 ]; then
  decide false "git diff $first HEAD failed (status $status)."
fi

if ! runs=$("$gh_cmd" api "repos/$repo/commits/$second/check-runs?check_name=${check// /%20}&filter=all&per_page=100"); then
  decide false "Could not read the check runs on $second."
fi
# The newest run of that name from GitHub Actions: a re-run replaces an
# earlier result, and another app can't stand in for the workflow's check.
if ! conclusion=$(jq -r '[.check_runs[] | select(.app.slug == "github-actions")]
    | sort_by(.started_at // "", .id) | last | .conclusion // "missing"' <<<"$runs"); then
  decide false "Could not parse the check runs on $second."
fi
if [ "$conclusion" != success ]; then
  decide false "$check on the PR head $second is $conclusion, not success."
fi

decide true "The merge $head has the tree of its PR head $second, where $check passed, and no cache-key input changed since $first."
