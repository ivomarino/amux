#!/usr/bin/env bash
# AEAB-19 — the frustrations.md gate must be able to FAIL, with no board reachable.
#
# It could not. Two independent defects in scripts/frustrations_audit.py, and each one
# alone was enough to make `checks` — the only required status check on main — green
# over a broken file:
#
#   1. the board-unreachable branch returned a bare `2`, discarding whether `problems`
#      was non-empty. CI never has a board (checks.yml says so in its own comment) and
#      treats 2 as a pass, so per-entry structural findings were printed and thrown away
#      on every push.
#   2. `structure_ok = structure_check(...)` was assigned and never read. The drift check
#      whose docstring records it stopping a live incident from being queued for DELETION
#      was advisory by accident.
#
# Both were live. Commit 18590ca8 landed an entry with no `## ` heading — the parser folds
# such a block into the preceding entry — and main went green over it. One defect hid the
# other, which is why this test asserts each failure mode separately rather than just
# "the audit exits non-zero on a bad file".
#
# EVERY case runs with the board pointed at a closed port, because that is CI's condition
# and the condition under which the gate was broken. A version of this test that ran
# against a live board would pass against the OLD code and prove nothing.
#
# The audit resolves frustrations.md as `Path(__file__).parent.parent/frustrations.md`, so
# each case builds a throwaway repo (scripts/ + frustrations.md) and copies the REAL
# script into it — the shipped decision path, not a paraphrase.
#
# Exit 0 = all pass, 1 = a failure. Wired into .github/workflows/checks.yml.
set -uo pipefail
cd "$(dirname "$0")/.."
AUDIT="$(pwd)/scripts/frustrations_audit.py"
REAL_FRUST="$(pwd)/frustrations.md"
PASS=0; FAIL=0
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

# Board unreachable for every case: port 9 is reserved/discard.
export AMUX_URL="https://127.0.0.1:9"
export AMUX_API="https://127.0.0.1:9"

# A conforming entry, used as the base that each defect case mutates. Kept in one place
# so a contract change breaks this file loudly instead of leaving stale fixtures.
good_entry() {
  cat <<'EOF'
## a well-formed entry
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-17
SESSION: test
CARD: none
SYMPTOM: something observable happened
COST: 1 minute
FIX: do the thing
EOF
}

# Build a throwaway repo whose frustrations.md is whatever arrives on stdin, run the
# REAL audit inside it, echo the exit code.
run_on() { # $1 = case dir name
  local d="$TMP/$1"
  mkdir -p "$d/scripts"
  cp "$AUDIT" "$d/scripts/frustrations_audit.py"
  cat > "$d/frustrations.md"
  ( cd "$d" && python3 scripts/frustrations_audit.py >"$d/out.txt" 2>&1; echo $? )
}

check_rc() { # label expected actual casedir
  if [ "$2" = "$3" ]; then PASS=$((PASS+1));
  else
    FAIL=$((FAIL+1)); echo "FAIL: $1 — expected exit $2, got $3"
    [ -f "$TMP/$4/out.txt" ] && sed 's/^/      /' "$TMP/$4/out.txt" | head -6
  fi
}
check_says() { # label needle casedir
  if grep -qF "$2" "$TMP/$3/out.txt" 2>/dev/null; then PASS=$((PASS+1));
  else FAIL=$((FAIL+1)); echo "FAIL: $1 — output did not mention '$2'"; fi
}

header() { printf '# frustrations\n\nblurb\n\n---\n\n'; }

# ---------------------------------------------------------------------------
# (a) THE CONTROL, and it comes first on purpose. A well-formed file must exit 2
#     — board unchecked, nothing structurally wrong. If this ever fails, every
#     "exits 1" assertion below is meaningless, because a script that exits 1 on
#     everything would satisfy them all. This is also what proves the fix did not
#     simply turn the gate into an unconditional failure.
# ---------------------------------------------------------------------------
rc=$( { header; good_entry; } | run_on control )
check_rc "(a) CONTROL: conforming file exits 2 (board unchecked, nothing wrong)" 2 "$rc" control

# ---------------------------------------------------------------------------
# (b) The regression that motivated all of this: a missing required field, board
#     unreachable. Was exit 2 (pass). Must be exit 1.
# ---------------------------------------------------------------------------
rc=$( { header; good_entry | grep -v '^SEVERITY:'; } | run_on missing_severity )
check_rc   "(b) missing SEVERITY fails even with no board" 1 "$rc" missing_severity
check_says "(b) names the missing field" "SEVERITY" missing_severity
check_says "(b) still reports the board was unreachable" "CANNOT REACH BOARD" missing_severity

# ---------------------------------------------------------------------------
# (c) A second required field, to prove (b) is not special-cased.
# ---------------------------------------------------------------------------
rc=$( { header; good_entry | grep -v '^SYMPTOM:'; } | run_on missing_symptom )
check_rc   "(c) missing SYMPTOM fails even with no board" 1 "$rc" missing_symptom
check_says "(c) names the missing field" "SYMPTOM" missing_symptom

# ---------------------------------------------------------------------------
# (d) THE 18590ca8 SPECIMEN, rebuilt from the incident rather than invented: an
#     entry appended with no `## ` heading. The parser folds it into the previous
#     entry, so no field is "missing" — the only signal is the DATE/STATUS count
#     disagreeing with the heading count, which is precisely the check whose
#     result was being discarded. A test built only from missing fields (b, c)
#     would pass while this half stayed broken.
# ---------------------------------------------------------------------------
rc=$( { header; good_entry; printf '\n---\nDATE: 2026-08-17\nSTATUS: open\nAREA: attribution\n'; } | run_on headless )
check_rc   "(d) a heading-less entry fails via the drift check" 1 "$rc" headless
check_says "(d) reports the drift, not a missing field" "STRUCTURE DRIFT" headless

# ---------------------------------------------------------------------------
# (e) THE REAL FILE must pass, or this PR turns main red the moment it lands.
#     Fixing a gate is a migration event; this is the assertion that says the
#     migration was actually completed rather than merely intended.
# ---------------------------------------------------------------------------
rc=$( run_on realfile < "$REAL_FRUST" )
check_rc "(e) the repo's own frustrations.md passes (main stays green)" 2 "$rc" realfile

echo
echo "test-frustrations-audit: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
