#!/usr/bin/env bash
# What am I actually reviewing? Run this BEFORE the verification command.
#
# On a shared checkout the worktree is not the commit. ~50 lanes edit these
# files continuously, so a reviewer can run exactly the right command against
# bytes that are not the fix: a peer's uncommitted edit on top, or a sha that
# never made it into this history. Neither shows up in the command's output,
# so the review passes or fails for a reason nobody can see.
#
# Established as a habit by amux-cloud while reviewing AC-422, who checked both
# before running anything and said so. This script exists because a habit that
# depends on the reviewer remembering reaches whoever remembers (ethos rule 1),
# and the whole point of a review recipe is that it reaches the next person.
#
#   scripts/review-preflight.sh <sha> [path ...]
#
# Exit 0 only when BOTH hold: the named paths match the sha in the worktree,
# and the sha is in this checkout's history.
set -uo pipefail

sha="${1:-}"
shift || true
if [ -z "$sha" ]; then
  echo "usage: scripts/review-preflight.sh <sha> [path ...]" >&2
  exit 2
fi

rc=0

# 1. Is the sha even in this history? A reviewer handed a sha from another clone,
#    or from a branch that was never merged here, gets a clean-looking diff
#    against nothing.
if git merge-base --is-ancestor "$sha" HEAD 2>/dev/null; then
  echo "ok    $sha is in this checkout's history (ancestor of HEAD)"
else
  echo "FAIL  $sha is NOT an ancestor of HEAD here. You are not reviewing this history."
  rc=1
fi

# 2. Do the files on disk match that sha? Computed per path and PRINTED per path,
#    never summarised: a count cannot say which file a peer is mid-edit in.
if [ "$#" -eq 0 ]; then
  echo "note  no paths given, so worktree drift was NOT checked. Pass the files under review."
else
  drifted=0
  unmeasured=0
  for p in "$@"; do
    # READ THE STATUS, NOT JUST THE OUTPUT. `git diff <bad-sha>` exits non-zero
    # and prints NOTHING, and an emptiness test reads that as "matches" -- a
    # confident finding about the FILE produced by an instrument that never ran.
    # Caught in this script by testing it with a sha that does not exist, which
    # reported "ok  <path> matches 0000000...". That is exactly AF-559: any check
    # whose population comes from an instrument that can be unavailable will,
    # when it is, report a clean answer about the subject.
    out=$(git diff "$sha" -- "$p" 2>/dev/null); st=$?
    if [ "$st" -ne 0 ]; then
      echo "UNMEASURED  $p not compared: git diff against $sha failed (exit $st)"
      unmeasured=$((unmeasured + 1))
    elif [ -n "$out" ]; then
      echo "FAIL  $p differs from $sha in this worktree (peer edit on top, or your own)"
      drifted=$((drifted + 1))
    else
      echo "ok    $p matches $sha"
    fi
  done
  # The summary takes a variable, so it cannot disagree with the lines above.
  # Both numbers, always, and both from variables. "0 drifted" beside a silent
  # 3-unmeasured is the sentence this script exists to stop anyone reading.
  echo "      $drifted of $# path(s) drifted; $unmeasured of $# NOT MEASURED"
  [ "$drifted" -gt 0 ] && rc=1
  [ "$unmeasured" -gt 0 ] && rc=1
fi

exit "$rc"
