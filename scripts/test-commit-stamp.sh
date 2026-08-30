#!/usr/bin/env bash
# Cells for the commit-message stamp (AMUX-3916).
#
# WHY THIS EXISTS. `Amux-Session` is read from $AMUX_SESSION, which is an
# ENVIRONMENT VARIABLE and therefore travels to every child process. Any process
# that inherits it — a subagent, a script, a session that wandered into this
# checkout — writes commits indistinguishable from that lane's. `Amux-Conversation`
# is a LOOKUP of that same variable, so it cannot corroborate it: a wrong stamp
# produces a wrong conversation id identically and the pair reads as doubly
# confirmed. Measured on 2026-08-30: four commits stamped to a lane that did not
# make them, and two agents citing the two fields to each other as agreeing
# sources.
#
# `Amux-Agent` is walked from the hook's own PROCESS ANCESTRY, which no env var
# can move. The property below is the whole point and it is stated as an
# invariance: change $AMUX_SESSION to anything you like and the agent field does
# not move.
#
# Runs the SHIPPED hook, not a retyped copy.
set -uo pipefail
cd "$(dirname "$0")/.."
HOOK="${COMMIT_STAMP_HOOK:-$(pwd)/scripts/git-hooks/prepare-commit-msg}"
PASS=0; FAIL=0
ok(){ PASS=$((PASS+1)); printf '  ok   %s\n' "$1"; }
no(){ FAIL=$((FAIL+1)); printf '  FAIL %s\n     %s\n' "$1" "${2:-}"; }

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
run(){ # run <session> -> prints the trailer block
  printf 'subject\n' > "$TMP/msg"
  AMUX_SESSION="$1" AMUX_HOME="$TMP/home" bash "$HOOK" "$TMP/msg" >/dev/null 2>&1
  grep '^Amux-' "$TMP/msg" 2>/dev/null
}
mkdir -p "$TMP/home/sessions"

echo "commit-stamp cells (AMUX-3916)"

# 1. THE INVARIANCE. Two different claimed lanes, one real committing process.
a="$(run lane-alpha | grep '^Amux-Agent:')"
b="$(run lane-beta  | grep '^Amux-Agent:')"
sa="$(run lane-alpha | grep '^Amux-Session:')"
sb="$(run lane-beta  | grep '^Amux-Session:')"
[ "$sa" != "$sb" ] \
  && ok "Amux-Session follows \$AMUX_SESSION (it is the claim)" \
  || no "Amux-Session invariant" "both runs said '$sa'; the test is not exercising the spoof"
if [ -n "$a" ] && [ "$a" = "$b" ]; then
  ok "Amux-Agent does NOT move with \$AMUX_SESSION ($a)"
else
  no "Amux-Agent must be invariant under \$AMUX_SESSION" "alpha='$a' beta='$b'"
fi

# 2. IT NAMES A REAL PROCESS, not a placeholder. A field that is merely PRESENT
#    looks identical to one that discriminates, which is the failure this whole
#    card is about.
pid="$(printf '%s' "$a" | sed -n 's/.*pid=\([0-9]\{1,\}\).*/\1/p')"
if [ -n "$pid" ] && ps -p "$pid" >/dev/null 2>&1; then
  ok "Amux-Agent pid=$pid is a live process"
else
  no "Amux-Agent must name a live pid" "got '$a'"
fi

# 3. REGRESSION: A PATH IS NOT A PROGRAM. The first draft matched `*claude*`
#    against the whole command line and picked up a shell whose cwd was
#    /private/tmp/claude-501/..., reporting that shell as the agent with
#    model=unspecified. Match the first token's basename.
if grep -q 'case "${_exe##\*/}" in' "$HOOK" || grep -q '_exe##' "$HOOK"; then
  ok "matches the executable's basename, not a substring of the command line"
else
  no "the agent walk must not glob \*claude\* over the whole command line" \
     "a cwd containing 'claude' would be reported as the agent"
fi

# 4. A CONVERSATION ID THE LANE HAS NOT CONFIRMED IS OMITTED (AMUX-3897).
#    An absent field reads as unknown; a wrong one reads as fact.
printf '{"cc_conversation_id":"11111111-2222-4333-8444-555555555555"}' \
  > "$TMP/home/sessions/unconfirmed.meta.json"
if run unconfirmed | grep -q '^Amux-Conversation:'; then
  no "an unconfirmed conv id must not be stamped" "$(run unconfirmed)"
else
  ok "unconfirmed conversation id is omitted, not guessed"
fi
#    CONTROL: a freshly confirmed one IS stamped, or cell 4 passes by the field
#    never being emitted at all.
python3 - "$TMP/home/sessions/confirmed.meta.json" <<'PY'
import json,sys,time
json.dump({"cc_conversation_id":"11111111-2222-4333-8444-555555555555",
           "cc_conversation_confirmed_at":int(time.time())}, open(sys.argv[1],"w"))
PY
if run confirmed | grep -q '^Amux-Conversation:'; then
  ok "a freshly confirmed conversation id IS stamped (cell 4 is not vacuous)"
else
  no "a confirmed conv id must still be stamped" "$(run confirmed)"
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
