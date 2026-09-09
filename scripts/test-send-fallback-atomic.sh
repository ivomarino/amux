#!/usr/bin/env bash
# AF-624 — the raw-tmux fallback must submit its body and Enter without a window
# another writer can reach.
#
# THE DEFECT: `send-keys -l "$body"`, `sleep 0.05`, `send-keys Enter` — three
# calls, and anything reaching the pane in that 50ms is submitted as part of this
# message by OUR Enter. Observed live 2026-09-08, tubescience -> mvs-infra: a
# directive reading "You OWN the A->F arc... git pull --rebase..." arrived
# concatenated mid-sentence, wearing the sender's apparent voice. It only runs
# when the server is unreachable, so every occurrence is during an incident,
# which is when a misattributed instruction is least likely to be double-checked.
#
# MEASURED before the fix, 80 concurrent injections into a real pane:
#   two calls + sleep   40 lines, 40 of 40 SPLICED
#   one command list    80 lines,  0 spliced
#
# Exit 0 = all pass, 1 = a failure.
set -euo pipefail
cd "$(dirname "$0")/.."
PASS=0; FAIL=0; SKIP=0

# (a) SOURCE SHAPE — runs anywhere, including a CI box with no tmux. The body
#     and the Enter must be ONE command list, and no sleep may separate them.
src="$(cat ./amux)"
if [[ "$src" == *'-l "$marked" \; send-keys'* ]]; then
  PASS=$((PASS+1))
else
  FAIL=$((FAIL+1)); echo "FAIL shape: body and Enter are not one tmux command list" >&2
fi
# The old form is specifically what must not come back.
if [[ "$src" != *'-l "$marked" 2>/dev/null; then'* ]]; then
  PASS=$((PASS+1))
else
  FAIL=$((FAIL+1)); echo "FAIL shape: the two-call form is back" >&2
fi

# (b) THE MECHANISM, not the wiring. This drives tmux DIRECTLY with the same
#     command-list form the CLI now uses, because the fallback only runs when the
#     server is unreachable and standing that fixture up is a bigger apparatus
#     than the thing under test.
#
#     SO READ THE TWO CELLS TOGETHER AND DO NOT OVER-TRUST THIS ONE: (a) pins
#     that amux uses a single command list, (b) pins that a single command list
#     is in fact splice-free on this tmux. Reverting the CLI to the two-call form
#     reddens (a) and NOT this cell, which is correct and is exactly the limit
#     worth knowing. Neither alone is the claim.
#
#     Needs tmux; SKIPS LOUDLY with the reason rather than passing quietly,
#     because a skipped concurrency test that reads as green is how this defect
#     survives a rewrite.
if ! command -v tmux >/dev/null 2>&1; then
  SKIP=$((SKIP+1))
  echo "SKIP concurrency: tmux is not installed on this host, so the only cell that can OBSERVE a splice did not run" >&2
else
  T="af624test-$$"; OUT="$(mktemp)"
  tmux new-session -d -s "$T" -x 400 -y 50 \
    "while IFS= read -r l; do printf '%s\n' \"\$l\" >> $OUT; done" 2>/dev/null
  sleep 0.4
  inject() {
    local body="[FENCE-$1] $(printf 'x%.0s' $(seq 1 120)) [END-$1]"
    tmux send-keys -t "=$T:" -l "$body" \; send-keys -t "=$T:" Enter 2>/dev/null
  }
  for _ in $(seq 1 25); do inject A & inject B & wait; done
  sleep 0.6
  tmux kill-session -t "$T" 2>/dev/null
  # `grep -c` EXITS 1 when the count is zero, having already printed "0". So
  # `$(grep -c ... || echo 0)` yields "0\n0" and every arithmetic test on it is a
  # syntax error, which is how the first run of this file reported a splice
  # failure over a clean pane. Assign, then default on the failure status.
  lines=$(grep -c . "$OUT" 2>/dev/null) || lines=0
  spliced=$(grep -c 'FENCE-A.*FENCE-B\|FENCE-B.*FENCE-A' "$OUT" 2>/dev/null) || spliced=0

  # THE POSITIVE CONTROL, and it is the cell that makes the next line mean
  # something: if nothing was submitted at all, `spliced=0` is vacuously true and
  # a completely broken fallback reports clean.
  if [[ "$lines" -ge 40 ]]; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1)); echo "FAIL control: only $lines line(s) submitted from 50 injections; a zero splice count over nothing proves nothing" >&2
  fi
  if [[ "$spliced" -eq 0 ]]; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1)); echo "FAIL splice: $spliced of $lines submitted line(s) carried BOTH senders' fences" >&2
  fi
  rm -f "$OUT"
fi

printf 'send fallback atomicity: %d passed, %d failed, %d skipped\n' "$PASS" "$FAIL" "$SKIP"
[[ "$FAIL" -eq 0 ]]
