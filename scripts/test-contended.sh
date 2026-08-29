#!/usr/bin/env bash
# Run a cargo test command and say whether a BUILD WAS IN FLIGHT while it ran.
#
# WHY (AMUX-3853). On 2026-08-28 a full `cargo test -p amux-server --lib` came
# back with 8 failures in `opencode::structured`, in code nobody had touched.
# Re-run in isolation: 15 pass, 0 fail. The failures were contention — those
# tests spawn a binary out of the shared CARGO_TARGET_DIR while the auto-builder
# is rewriting it, the ETXTBSY family `2618b7d3` already added a retry for. The
# retry is not enough under the load this box actually carries (the fleet, a
# builder rebuilding on every commit, and any peer running clippy).
#
# The cost is not the wasted run. It is that "1530 pass, 0 failed" and "8 failed"
# are both produced by the same command against the same code, and NOTHING in
# cargo's output says which kind of run you got. Every green suite here silently
# means "green, AND nothing was building" — the second clause is invisible, so
# nobody states it, and a red one gets read as a regression.
#
# The wrong lesson from that is "ignore red suites". This exists so you do not
# have to: it prints the missing clause beside the result.
#
#   scripts/test-contended.sh -p amux-server --lib autofix
#
# Exit status is the test command's, untouched — this reports, it never decides.
set -uo pipefail

LOCK="${AMUX_RS_BUILD_LOCK:-$HOME/.amux/rust-build.lock}"
: "${CARGO_TARGET_DIR:=$HOME/.amux/rust-build-target}"
export CARGO_TARGET_DIR

# Sampled, not checked once at the start and once at the end. A build that
# starts AND finishes inside a two-minute suite is invisible to the endpoints
# and is exactly the run that produces a confusing red.
SEEN=0
OWNERS=""
sample() {
  while :; do
    if [ -d "$LOCK" ]; then
      SEEN=1
      p=$(cat "$LOCK/pid" 2>/dev/null || echo "?")
      case " $OWNERS " in *" $p "*) ;; *) OWNERS="$OWNERS $p" ;; esac
      printf '%s\n' "$p" >> "$FLAG"
    fi
    sleep 2
  done
}

FLAG=$(mktemp)
trap 'kill "$SAMPLER" 2>/dev/null; rm -f "$FLAG"' EXIT INT TERM

sample & SAMPLER=$!

cargo test "$@"
RC=$?

kill "$SAMPLER" 2>/dev/null
wait "$SAMPLER" 2>/dev/null

if [ -s "$FLAG" ]; then
  builds=$(sort -u "$FLAG" | tr '\n' ' ' | sed 's/ *$//')
  echo ""
  echo "contention: A BUILD WAS IN FLIGHT during this run (builder pid(s): $builds)."
  echo "contention: a failure here may be ETXTBSY on the shared target dir rather than"
  echo "contention: a regression. Re-run the failing module alone before believing it"
  echo "contention: (AMUX-3853)."
else
  # SAID EXPLICITLY, not left as silence. "No line printed" would be
  # indistinguishable from "this script did not run", which is the same
  # absent-versus-measured confusion the whole entry is about.
  #
  # NAMES THE AUTO-BUILDER, not "a build" (2026-08-29). This arm used to read
  # "no build was in flight", and the first real run of this script printed it
  # directly under cargo's own "Compiling amux-server ... Finished in 1m 04s".
  # Both were true and the sentence still read as false, because what is
  # sampled is $LOCK, the AUTO-BUILDER's lock: the hazard is another process
  # rewriting the shared binary underneath a test that spawns it, not the
  # compile this very command is doing. A probe has to say what it measured,
  # or the one line that exists to settle "real or contention?" becomes the
  # thing you have to go and check.
  # SAYS WHAT IT RULED OUT, not "a failure here is real" (2026-08-29, second
  # pass). That phrasing was fixed once already this morning for naming "a
  # build" when it samples the AUTO-BUILDER, and it was still overclaiming in a
  # second dimension: a reader takes "real" to mean "a code regression", and
  # this script only ever knew about ONE environmental cause.
  #
  # The specimen arrived the same day. A full lib suite came back 1552 passed /
  # 6 failed under this exact clean verdict, and all six were host memory
  # pressure — swap at 8700MB over the 8192MB AMUX_MEM_SWAP_DENY_MB threshold,
  # so worker start was refused 503 where the tests expect 202. Real failures,
  # nothing to do with the code under test, and this line called them real.
  #
  # An instrument that rules out one cause has to say WHICH, or the next reader
  # generalises it to all of them. Which is the whole argument the top of this
  # file makes about plain `cargo test`, arriving one level up.
  echo ""
  echo "contention: the auto-builder was NOT rebuilding during this run, so the shared"
  echo "contention: binary was stable under it. A failure here is NOT build contention."
  echo "contention: (Cargo's own compile for this command is not the hazard; a peer's is.)"
  echo "contention: THAT IS THE ONLY THING RULED OUT. Host pressure still fails tests that"
  echo "contention: start workers — check the failure body for a 503 admission refusal"
  echo "contention: before reading a red as a regression."
fi

exit "$RC"
