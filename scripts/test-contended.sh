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

# THE SECOND WAY A PEER REDDENS YOUR SUITE (AF-356).
#
# The builder is one of TWO causes and this wrapper only ever measured that one,
# so its clean verdict ("NOT build contention") read as "therefore your bug". The
# other cause is a peer's UNCOMMITTED SOURCE sitting in the shared worktree: cargo
# compiles the tree, not your commit, so their half-finished edit fails a test in a
# module you never opened, and it passes on a rerun after they finish. Identical
# symptom to ETXTBSY, and nothing distinguished them.
#
# Measured live 2026-08-31: `amux` ran the suite and got one failure in
# `gate_table_matches_python`, which they had not touched. They read this
# wrapper's clean verdict, concluded ETXTBSY, and carried that into a
# verification request as its stated weakest evidence line. The real cause was my
# uncommitted `ItemType::Decision` in the shared tree. Both of us were reasoning
# from an instrument that answered a narrower question than the sentence it printed.
#
# Captured BEFORE and AFTER, because a tree that CHANGED under the compile is the
# strongest form of the signal and a single snapshot cannot see it.
#
# Deliberately NOT attributed to a lane. Owner-by-mtime is the inference that has
# been wrong repeatedly on this checkout (AF-179, AMUX-3662, where a lane's own
# writes read as a phantom co-editor), and a confident wrong owner is worse than a
# named file with no owner. The file names are what a reader needs; they can tell
# in one second whether a path is theirs.
dirty_now() { git status --porcelain --untracked-files=no 2>/dev/null | awk '{print $NF}' | sort; }
DIRTY_BEFORE=$(dirty_now)

cargo test "$@"
RC=$?

DIRTY_AFTER=$(dirty_now)

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

# THE WORKTREE CLAUSE — printed on BOTH arms, never only the clean one.
#
# A caveat that lives inside one branch is a caveat the other branch's reader
# never sees (ethos rule 1: a statement about a whole set belongs at the top
# level, not inside one arm). A dirty tree explains a red just as well when the
# builder WAS running, and reading only the ETXTBSY note would stop the search
# one cause short.
if [ -n "$DIRTY_BEFORE" ] || [ -n "$DIRTY_AFTER" ]; then
  n_before=$(printf '%s\n' "$DIRTY_BEFORE" | grep -c . || true)
  n_after=$(printf '%s\n' "$DIRTY_AFTER" | grep -c . || true)
  echo "worktree:  $n_before uncommitted file(s) at start, $n_after at end."
  if [ "$DIRTY_BEFORE" != "$DIRTY_AFTER" ]; then
    echo "worktree:  THE TREE CHANGED DURING THIS RUN. cargo compiled the worktree, not"
    echo "worktree:  your commit, so a file that moved under the compile can fail a test"
    echo "worktree:  in a module you never opened. This is the strongest form of the signal."
    printf '%s\n%s\n' "$DIRTY_BEFORE" "$DIRTY_AFTER" | sort -u | sed 's/^/worktree:    /'
  else
    printf '%s\n' "$DIRTY_AFTER" | sed 's/^/worktree:    /'
  fi
  echo "worktree:  Any of these — yours OR a peer's — can redden a module you did not"
  echo "worktree:  touch. Owner is NOT inferred here: mtime-based attribution has been"
  echo "worktree:  wrong on this checkout before (AF-179), and a confident wrong owner is"
  echo "worktree:  worse than a named file with none. You can tell which are yours."
else
  echo "worktree:  clean at start and end, so no peer's uncommitted source was in this"
  echo "worktree:  build. Stated because a silent probe and a clean tree look identical."
fi

exit "$RC"
