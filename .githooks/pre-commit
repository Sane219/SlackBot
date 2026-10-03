#!/usr/bin/env bash
# Pre-commit. Runs the same script CI runs, so the two cannot disagree.
#
# Cheap on purpose: fmt, clippy and the unit tests, plus the one check with no automated
# equivalent — is `dist/` current. The e2e suite and the browser checks need a listening
# server and are left to CI; `scripts/check.sh --full` runs those when a server is up.
#
# Bypass with `--no-verify` when you are mid-refactor and will fix it on the next commit.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Nothing staged, nothing to check.
if git diff --cached --quiet; then
  exit 0
fi

# `scripts/check.sh` inspects the working tree for staleness, which is the right question
# pre-commit: the point is that what you are about to commit is consistent.
exec scripts/check.sh