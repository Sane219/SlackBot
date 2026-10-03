#!/usr/bin/env bash
# What CI runs, in the order CI runs it.
#
# One script, called by both the pre-commit hook and the workflow, because the two drifted
# once already: the browser checks sat in a job that never started a server, so they
# failed on every run while the commit that added them claimed they worked. If the hook and
# CI cannot disagree, they cannot.
#
#   scripts/check.sh          # everything that needs no network
#   scripts/check.sh --full   # also the e2e suite and the browser checks (needs a server)
set -euo pipefail
cd "$(dirname "$0")/.."

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
step() { bold "→ $*"; }

fail=0
run() {
  local label="$1"
  shift
  step "$label"
  if "$@"; then
    return 0
  fi
  echo "FAILED: $label" >&2
  fail=1
}

run "fmt"      cargo fmt --all -- --check
run "clippy"   cargo clippy --all-targets --all-features
run "test"     cargo test

# ADR-0009: `dist/` is committed, so it is a contract. A UI change that does not
# regenerate it ships a binary serving the previous page — a blank screen, not an error.
if ! git diff --quiet -- dist ui/src ui/package-lock.json 2>/dev/null; then
  step "dist is stale relative to ui/src"
  echo "run 'npm --prefix ui run build' and commit dist/ alongside the source change." >&2
  fail=1
else
  step "dist matches ui/src"
fi

if [ "${1:-}" = "--full" ]; then
  # These need a listening server, so the caller starts one; see .github/workflows/ci.yml.
  run "e2e" env SLACKBOT_E2E_URL=http://127.0.0.1:7321 \
    cargo test --test e2e -- --ignored --test-threads=1
  run "browser" npm --prefix ui run verify
fi

if [ "$fail" -ne 0 ]; then
  echo >&2
  echo "checks failed" >&2
  exit 1
fi
bold "all checks passed"