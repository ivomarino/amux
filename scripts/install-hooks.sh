#!/bin/bash
# Install the repo's git hooks (secret scan + syntax checks + cross-session
# staged-guard before every commit).  Run once after cloning:
#   ./scripts/install-hooks.sh
#
# BOTH files are installed, not just the hook. The guard used to live only in
# .git/hooks/ — untracked — so a fresh clone got a pre-commit whose guard call
# found nothing beside it and silently proceeded with cross-session sweep
# protection OFF (2026-08-06). Installing the caller without the callee is the
# shape that made that invisible.
set -e
ROOT="$(git rev-parse --show-toplevel)"

install -m 0755 "$ROOT/scripts/git-hooks/pre-commit"        "$ROOT/.git/hooks/pre-commit"
install -m 0755 "$ROOT/scripts/git-hooks/amux-staged-guard" "$ROOT/.git/hooks/amux-staged-guard"

# Verify rather than announce (ethos #7): report what is actually in place, so a
# stale installed copy — which is how the AC-239 secret patterns sat inactive
# locally while CI had them — cannot hide behind a success message.
for f in pre-commit amux-staged-guard; do
  if cmp -s "$ROOT/scripts/git-hooks/$f" "$ROOT/.git/hooks/$f"; then
    echo "  ok   .git/hooks/$f matches scripts/git-hooks/$f"
  else
    echo "  FAIL .git/hooks/$f differs from scripts/git-hooks/$f" >&2
    exit 1
  fi
done
echo "hooks installed: secret scan + Python/JS syntax + cross-session staged-guard."
