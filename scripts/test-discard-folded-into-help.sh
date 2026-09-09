#!/usr/bin/env bash
# AF-635 — `--folded-into` must be findable by a lane that goes looking.
#
# THE DEFECT: the flag has worked since 99763834 and appeared on NEITHER help
# surface. gtm-engine hit three specimens in one session where a hand-discarded
# capture shell produced "closed the request without resolving the dependency ...
# evidence: not recorded" while the work sat safely on a sibling card, and when
# they went looking for the flag that fixes it they found nothing. A capability
# nobody can name is a capability nobody has (ethos rule 1).
#
# ASSERTS ON RENDERED OUTPUT, because these are heredocs and a swallowed
# backtick or a dropped continuation is invisible in the source (AF-621).
#
# Exit 0 = all pass, 1 = a failure.
set -uo pipefail
cd "$(dirname "$0")/.."
AMUX_BIN="${AMUX_BIN:-./amux}"
PASS=0; FAIL=0
check()  { if [[ "$2" == *"$3"* ]]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); printf 'FAIL %s: missing %q\n' "$1" "$3" >&2; fi; }

# (a) THE VERB LIST — where a lane looks before it guesses.
verbs="$("$AMUX_BIN" board --help 2>&1 || true)"
check verbs-flag   "$verbs" "--folded-into <ID>"
check verbs-why    "$verbs" "work moved to another"

# (b) THE UNKNOWN-FLAG USAGE — where a lane lands when it guesses wrong.
usage="$("$AMUX_BIN" board discard AF-0 --nosuchflag 2>&1 || true)"
check usage-flag   "$usage" "--folded-into <ID>"
check usage-scope  "$usage" "discard only"

# (c) THE FLAG STILL WORKS. Help that documents a flag the parser rejects is
#     worse than silence, so this drives the real dispatch path. It must NOT
#     die on the flag itself; AMUX_API points nowhere so nothing mutates a board.
out="$(AMUX_API='https://127.0.0.1:1' "$AMUX_BIN" board discard AF-0 --folded-into AF-1 2>&1 || true)"
if [[ "$out" == *"unknown flag"* ]]; then
  FAIL=$((FAIL+1)); printf 'FAIL parser: help names --folded-into but the parser rejects it\n' >&2
else
  PASS=$((PASS+1))
fi

# (d) POSITIVE CONTROL. If --help stopped emitting anything, (a) and (b) would
#     fail loudly, but a check that only ever greps for absence would not. This
#     pins that the surfaces are alive at all.
check verbs-alive "$verbs" "amux board discard"
check usage-alive "$usage" "Usage: amux board"

printf 'discard --folded-into help: %d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
