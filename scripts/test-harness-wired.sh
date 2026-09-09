#!/usr/bin/env bash
# test-harness-wired.sh — AF-346, one level up.
#
# AF-346's incident: the a99955f7 dashboard regression was caught by a guard that
# ALREADY EXISTED, was correct, and would have blocked the commit. It did not
# run. The fix made the RUNNER say what it skipped. This checks the layer above
# it, where the same failure is invisible for longer: a harness that nothing
# invokes at all.
#
# MEASURED at the time this was written: 80 scripts/test-* harnesses, 48 named by
# a workflow or a git-hook, 32 named NOWHERE. Among them was
# scripts/test-target-clause.sh, the harness written FOR AF-346, which had never
# run in CI. A guard nobody calls is indistinguishable from a guard that passes,
# and it is worse than no guard, because it is counted as coverage.
#
# THIS IS A RATCHET, NOT A SWEEP. Wiring 32 harnesses at once would turn a check
# nobody has run into a required gate for every lane on the same afternoon, and
# some of them compile. So the 32 are recorded as a BASELINE and this fails only
# on a harness that is unwired and NOT in it. Number 33 cannot arrive quietly;
# the existing 32 stay someone's deliberate decision instead of my sweep
# (ethos rule 8).
#
# The baseline also SHRINKS: an entry naming a file that no longer exists, or one
# that is now wired, is an error. A stale allowlist is how a ratchet becomes a
# rubber stamp.
set -euo pipefail
cd "$(dirname "$0")/.."

BASELINE=scripts/fixtures/harness-wired-baseline.txt
PASS=0
FAIL=0
say() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); echo "  ok    $1"
        else FAIL=$((FAIL+1)); echo "  FAIL  $1"; fi; }

# Every place a harness can legitimately be invoked from. Kept explicit rather
# than a repo-wide grep: a harness "referenced" only by a comment or by another
# harness's docstring is not wired, and a whole-repo grep would call it wired.
# scripts/test-selector-clauses.sh names test-target-clause.sh in a comment,
# which is exactly the false positive this avoids.
#
# COMMENTS ARE STRIPPED, and this file's first run is why. The line above used to
# `cat` the workflow whole, so the comment I wrote in checks.yml explaining that
# test-target-clause.sh is deliberately NOT wired made it read as wired, and the
# stale-baseline cell fired on it. A mention is not an invocation, which is the
# exact distinction the paragraph above claims and the first cut did not make.
# YAML comments and the bash comments inside a `run: |` block both start with #,
# and neither invokes anything, so one rule covers both.
strip_comments() { sed 's/[[:space:]]*#.*$//'; }
refs=$(mktemp); trap 'rm -f "$refs"' EXIT
cat .github/workflows/*.yml .github/workflows/*.yaml 2>/dev/null | strip_comments >> "$refs" || true
for f in scripts/git-hooks/*; do [ -f "$f" ] && strip_comments < "$f" >> "$refs"; done
[ -f .githooks/pre-push ] && strip_comments < .githooks/pre-push >> "$refs"
[ -f .githooks/pre-commit ] && strip_comments < .githooks/pre-commit >> "$refs"

unwired=""
n_total=0
for h in scripts/test-*.sh scripts/test-*.py scripts/test-*.mjs; do
  [ -e "$h" ] || continue
  n_total=$((n_total+1))
  b=$(basename "$h")
  grep -qF "$b" "$refs" || unwired="$unwired$b
"
done
unwired=$(printf '%s' "$unwired" | sed '/^$/d' | sort)

[ -f "$BASELINE" ] || : > "$BASELINE"
known=$(grep -vE '^\s*(#|$)' "$BASELINE" | awk '{print $1}' | sort)

# NEW UNWIRED — the thing this exists to stop.
new=$(comm -23 <(printf '%s\n' "$unwired") <(printf '%s\n' "$known") | sed '/^$/d')
if [ -n "$new" ]; then
  echo "  NEW UNWIRED HARNESS(ES) — written, never invoked:"
  printf '%s\n' "$new" | sed 's/^/      /'
  echo "      Wire it into .github/workflows/checks.yml, or add it to $BASELINE"
  echo "      with a one-line reason on the same line."
fi
say "no harness is unwired without a recorded reason" "$([ -z "$new" ] && echo 0 || echo 1)"

# STALE BASELINE — an allowlist that outlives its entries stops being a record.
gone=""
while read -r b; do
  [ -n "$b" ] || continue
  [ -e "scripts/$b" ] || gone="$gone $b"
done <<< "$known"
if [ -n "$gone" ]; then echo "  baseline names files that no longer exist:$gone"; fi
say "every baseline entry still exists" "$([ -z "$gone" ] && echo 0 || echo 1)"

fixed=$(comm -13 <(printf '%s\n' "$unwired") <(printf '%s\n' "$known") | sed '/^$/d')
if [ -n "$fixed" ]; then
  echo "  these are WIRED now and can leave the baseline:"
  printf '%s\n' "$fixed" | sed 's/^/      /'
fi
say "no baseline entry is already wired" "$([ -z "$fixed" ] && echo 0 || echo 1)"

n_unwired=$(printf '%s\n' "$unwired" | sed '/^$/d' | grep -c . || true)
echo ""
echo "harness-wired: $n_unwired of $n_total harness(es) are invoked by nothing ($(printf '%s\n' "$known" | grep -c . || true) recorded)"
echo "test-harness-wired: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
