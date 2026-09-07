#!/usr/bin/env bash
# AF-561 — THE RATCHET. Every scripts/test-*.sh that prints a success verdict must
# carry `set -e`, or a call to a helper that does not exist prints "command not
# found" to stderr and the suite still reports PASS.
#
# Fixing 43 harnesses (AF-560, AF-562) is worth nothing in six months if script 65
# is written without the guard and nothing says so. That absence is the same shape
# as the defect: something that did not run, reported as a clean result.
#
# Modelled on tests/dashboard_assets.rs, which pins that every name the dashboard
# CALLS exists and every name it DEFINES does not already.
#
# TWO CHECKS, and the second is why this is not just a grep:
#   STATIC   every verdict-printing harness carries `set -e`
#   DYNAMIC  the fixtures prove the mechanism, not just the text — an unguarded
#            script really does print PASS with a missing helper, and a guarded one
#            really does exit 127 printing nothing
#
# `-e` is the guard, deliberately NOT `command_not_found_handle`: that needs bash
# >= 4.0 and this box runs 3.2.57, where it never fires. One shipped inert on
# 2026-09-07 (40938593, reverted by ab50fe1d) and a detector that accepted it
# would have certified 40 dead guards as protection. `inert-handler.sh` is the
# fixture for exactly that.
set -euo pipefail
cd "$(dirname "$0")/.."

FIX=scripts/fixtures/harness-guard
CELLS=0
FAILED=0

check() {  # check <label> <condition-already-evaluated:0|1>
  CELLS=$((CELLS + 1))
  if [ "$2" -eq 0 ]; then echo "  ok    $1"
  else echo "  FAIL  $1"; FAILED=$((FAILED + 1)); fi
}

# Does this file print a success verdict at all? Both idioms: the literal
# `echo "PASS` (3 of 43 real harnesses) and the `PASS=0; FAIL=0` counter form,
# which is the COMMON one here. A detector built on the first alone reports the
# repo clean while 40 scripts are unguarded — that is measured, not hypothetical.
prints_verdict() { grep -qE '\bPASS\b' "$1"; }
# `-e` on the shebang-adjacent set line. Anchored to line start so a `set -e`
# inside a comment or a heredoc does not count.
is_guarded()     { grep -qE '^set -[a-zA-Z]*e' "$1"; }

echo "harness guard (AF-561)"
echo

# ── 1. THE DETECTOR, against fixtures whose answer is known ──────────────────
# Run FIRST and by design: a detector validated only against a clean repo passes
# because it found nothing to look at.
for f in unguarded-echo unguarded-counter inert-handler; do
  p=$FIX/$f.sh
  if prints_verdict "$p" && ! is_guarded "$p"; then r=0; else r=1; fi
  check "rejects $f.sh" "$r"
done
if prints_verdict "$FIX/guarded.sh" && is_guarded "$FIX/guarded.sh"; then r=0; else r=1; fi
check "accepts guarded.sh" "$r"

# ── 2. THE MECHANISM, not the text ──────────────────────────────────────────
# An unguarded script really does print a verdict with a missing helper...
out=$(bash "$FIX/unguarded-counter.sh" 2>/dev/null) || true
if printf '%s' "$out" | grep -q 'passed,'; then r=0; else r=1; fi
check "unguarded fixture really does print a verdict with a missing helper" "$r"

# ...and a guarded one really does abort, printing none.
if out=$(bash "$FIX/guarded-catches.sh" 2>/dev/null); then rc=0; else rc=$?; fi
check "guarded fixture exits 127 on a missing helper" "$([ "$rc" -eq 127 ] && echo 0 || echo 1)"
check "guarded fixture prints NO verdict" "$(printf '%s' "$out" | grep -qc 'passed,' >/dev/null 2>&1; printf '%s' "$out" | grep -q 'passed,' && echo 1 || echo 0)"

# ── 3. THE REPO ─────────────────────────────────────────────────────────────
unguarded=""
n_verdict=0
for f in scripts/test-*.sh; do
  [ -f "$f" ] || continue
  prints_verdict "$f" || continue
  n_verdict=$((n_verdict + 1))
  is_guarded "$f" || unguarded="${unguarded}${f}
"
done
n_unguarded=$(printf '%s' "$unguarded" | grep -c . || true)
check "every verdict-printing harness carries set -e" "$([ "$n_unguarded" -eq 0 ] && echo 0 || echo 1)"
if [ "$n_unguarded" -gt 0 ]; then
  echo "        $n_unguarded of $n_verdict print a verdict without \`set -e\`:"
  printf '%s' "$unguarded" | sed 's/^/          /'
  echo "        Add -e to the existing \`set -uo pipefail\` line, then RUN the script"
  echo "        before and after and compare its exit status AND cell count — set -e"
  echo "        changes behaviour in scripts not written for it (AF-562)."
fi

# A population of zero would make the cell above pass for the wrong reason.
check "the repo actually has verdict-printing harnesses to check" "$([ "$n_verdict" -gt 20 ] && echo 0 || echo 1)"

echo
echo "  population: $n_verdict verdict-printing harnesses, $n_unguarded unguarded"

# THE EXIT STATUS DEPENDS ON THE COUNT DIRECTLY, not only on the cell above.
#
# Measured: mutating that cell to a hardcoded 0 left this suite PASSING with an
# unguarded script present. The population line printed "1 unguarded" two lines
# above the word PASS and the run still exited 0 — the eye reads a summary as the
# RESULT of the lines above it rather than as an independent claim sitting beside
# them (CLAUDE.md's computed-not-written rule). A cell cannot police its own
# hardcoding, so the number gets a second, independent path to the exit status.
if [ "$n_unguarded" -ne 0 ]; then
  echo "FAIL ($n_unguarded unguarded harness(es) — exit forced by the count, not by a cell)"
  exit 1
fi

if [ "$FAILED" -eq 0 ]; then
  echo "PASS ($CELLS outcome cells)"
  exit 0
fi
echo "FAIL ($FAILED of $CELLS outcome cells)"
exit 1
