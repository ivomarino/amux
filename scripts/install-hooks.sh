#!/bin/bash
# Install the repo's git hooks. Run once after cloning:  ./scripts/install-hooks.sh
#
#   pre-commit         — secret scan + Rust/client-JS syntax, and the shim that
#                        calls the staged-guard below
#   amux-staged-guard  — cross-session sweep protection (AMUX-1730)
#
# WHY THIS NOW INSTALLS TWO FILES (2026-08-09). It used to install only
# pre-commit, on the grounds that amux-staged-guard was a GENERATED artifact:
# the Python server wrote it per work_dir and injected its own shim. That was
# right while python ran. amux-server.py is now deleted and nothing in the rust
# server generates the hook, so the "the server will install it" advice this
# script printed was false, and — worse — overwriting pre-commit here DELETED
# the shim the retired python had injected, turning the guard off while
# printing "ok ... matches".
#
# So the tracked copies are now the ONLY producer. If a generator ever comes
# back, remove them here in the same commit that adds it (the 2026-08-06 revert
# explains what a second producer costs).
set -e
ROOT="$(git rev-parse --show-toplevel)"

install -m 0755 "$ROOT/scripts/git-hooks/pre-commit" "$ROOT/.git/hooks/pre-commit"
install -m 0755 "$ROOT/scripts/git-hooks/amux-staged-guard" "$ROOT/.git/hooks/amux-staged-guard"

# Verify rather than announce (ethos #7): compare what landed against its source,
# so a stale installed copy cannot hide behind a success message. That drift was
# real and security-relevant — the AC-239 secret patterns (Clerk, R2, Slack,
# GitLab) sat in the tracked hook while .git/hooks/pre-commit was months old, so
# commits printed "Security scan passed" from a scanner that could match none of
# them.
fail=0
for h in pre-commit amux-staged-guard; do
  if cmp -s "$ROOT/scripts/git-hooks/$h" "$ROOT/.git/hooks/$h"; then
    echo "  ok   .git/hooks/$h matches scripts/git-hooks/$h"
  else
    echo "  FAIL .git/hooks/$h differs from scripts/git-hooks/$h" >&2
    fail=1
  fi
done

# The shim is the whole chain: an installed guard that pre-commit never calls is
# a file, not a guard. Check the LINK, not just the two files — this is exactly
# the failure that shipped when pre-commit was overwritten without it.
if grep -q 'amux-staged-guard' "$ROOT/.git/hooks/pre-commit" \
   && grep -q '"\$g" || exit 1' "$ROOT/.git/hooks/pre-commit"; then
  echo "  ok   pre-commit calls amux-staged-guard"
else
  echo "  FAIL pre-commit does NOT call amux-staged-guard — sweep protection is OFF" >&2
  fail=1
fi

# And the guard is only a guard if the SERVER routes its endpoint. Unrouted, it
# fails open and (before 2026-08-09) said nothing: POST /api/git/staged-guard
# answered 405 for the whole rust cutover, ~1,147 calls an hour, every one
# swallowed. Report it here, where someone is already looking at hooks.
URL="${AMUX_URL:-https://localhost:8824}/api/git/staged-guard"
code=$(curl -sk -o /dev/null -w '%{http_code}' -m 3 -X POST \
        -H 'Content-Type: application/json' -d '{}' "$URL" 2>/dev/null || echo 000)
case "$code" in
  400|200) echo "  ok   server routes POST /api/git/staged-guard (HTTP $code)" ;;
  000)     echo "  note amux server not reachable at $URL — could not check the endpoint" ;;
  404|405|501)
    # Loud, but NOT `fail=1`: the installer's exit code is a verdict on what it
    # installed, and a stale server is not something it can fix. The guard
    # itself now prints this on every commit, which is the enforcement point.
    echo "  WARN server does NOT route POST /api/git/staged-guard (HTTP $code) —" >&2
    echo "       the guard fails open on every commit until the amux server is updated." >&2 ;;
  *)       echo "  note POST /api/git/staged-guard answered HTTP $code" ;;
esac

[ "$fail" -eq 0 ] || exit 1
echo "hooks installed: secret scan + Rust/client-JS syntax + cross-session staged-guard."
