#!/usr/bin/env bash
# The raw-tmux send fallback must FENCE its body (AF-615, AF-624).
#
# WHY THIS EXISTS. When the server is unreachable, `amux send` falls back to
# typing the message into the target pane with `send-keys -l`, then sending
# Enter as a SEPARATE call. Anything that reaches the same pane in between is
# submitted as part of this message, by our Enter.
#
# Observed live, tubescience -> mvs-infra 2026-09-08 ~18:33Z: the delivered text
# had a STRAY TAIL concatenated mid-sentence, a directive reading "You OWN the
# A->F arc... git pull --rebase..." that was never in the sender's stdin and
# belongs to another pane. mvs-infra's harness refused it, which is the
# UNVERIFIED prefix (AF-455) doing its job. But a splice that can append text to
# a message wearing another lane's apparent voice is a way to deliver an
# instruction to run git commands, and nothing told the receiver where the real
# message ended.
#
# Neither the prefix nor the fence had any test at all before this file, which
# is why the fence could have been dropped by any later edit and nothing would
# have said so.
#
# It reads the SHIPPED `amux`, not a retyped copy (ethos rule 7).
set -euo pipefail
cd "$(dirname "$0")/.."
CLI=amux
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1"; echo "       $2"; }

# The single assignment that builds the injected body.
line="$(grep -n 'local marked=' "$CLI" | head -1 | cut -d: -f2-)"
if [ -z "$line" ]; then
  echo "  FAIL the fallback body assignment (local marked=) is gone from $CLI"
  echo "  0 passed, 1 failed"; exit 1
fi

# 1. It still marks itself unverified. This is AF-455 and it is what made the
#    live incident a refusal rather than an executed instruction.
case "$line" in
  *"UNVERIFIED INJECTION"*) ok "the body still marks itself as an unverified injection" ;;
  *) bad "the UNVERIFIED INJECTION prefix is gone" "$line" ;;
esac

# 2. It names the session it CLAIMS to be from, without asserting verification.
case "$line" in
  *'claims ${AMUX_SESSION:-unknown}'*) ok "it names the claimed sender without asserting it is verified" ;;
  *) bad "the claimed-sender clause is gone" "$line" ;;
esac

# 3. THE FENCE. The terminator must be named in the prefix AND appended after
#    the body. Naming it without appending it points at a marker that never
#    arrives; appending without naming leaves a receiver no reason to trust it.
named=0; appended=0
case "$line" in *"The message ends at [AMUX-INJECT-END]"*) named=1 ;; esac
case "$line" in *'$text [AMUX-INJECT-END]"'*) appended=1 ;; esac
if [ "$named" = 1 ] && [ "$appended" = 1 ]; then
  ok "the terminator is both named in the prefix and appended after the body"
else
  bad "the fence is incomplete (named=$named appended=$appended)" "$line"
fi

# 4. The instruction a receiver acts on: ignore anything past the marker.
case "$line" in
  *"must be ignored"*) ok "it tells the receiver what to do with a spliced tail" ;;
  *) bad "the ignore-after-marker instruction is gone" "$line" ;;
esac

# 5. CONTROL. The body must still be a SINGLE line: send-keys -l types
#    literally and the Enter is a separate call, so an embedded newline would
#    submit the prefix alone and strand the message. A fence that arrived by
#    breaking that would be worse than no fence.
if [ "$(printf '%s' "$line" | wc -l | tr -d ' ')" = "0" ]; then
  ok "the body is still a single line (an embedded newline would submit the prefix alone)"
else
  bad "the assignment spans lines; send-keys -l would submit early" "$line"
fi

# 6. CONTROL. The card this cites for the REMAINING work must not be a
#    discarded, unrelated one. The first version of this comment cited AF-613,
#    which is a peer's discarded capture about fold coverage: a stale citation
#    that still RESOLVES, so a reader follows it and lands somewhere else.
cite="$(grep -n 'pane-level exclusion' -A 1 "$CLI" | grep -oE 'AF-[0-9]+' | head -1)"
if [ "$cite" = "AF-624" ]; then
  ok "the follow-up citation points at the pane-exclusion card ($cite)"
else
  bad "the pane-exclusion follow-up cites '$cite', not AF-624" "check the comment above the fence"
fi

echo "  $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
