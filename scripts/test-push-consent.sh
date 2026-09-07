#!/usr/bin/env bash
# Proof for scripts/push-consent.sh (AF-548).
#
# The range c6876cf1..8a701877 is the real push this script was built from and
# it is permanent history on origin, so every number below is checkable by
# anyone. Deliberately asserts on the PURE-GIT half (commit counts, Rust/no-Rust
# split, which shas carry no Rust) rather than on live /api/sessions isolation,
# because a lane's isolation is a fact about today and would make this a test of
# the fleet rather than of the script.
#
# The PASS line is COMPUTED from the cell count, never written: a summary a
# script hardcodes cannot disagree with its own run, so it reads as measured
# while being unable to fail.
set -uo pipefail
cd "$(dirname "$0")/.."

BASE=c6876cf1
TIP=8a701877
CELLS=0
FAILED=0

check() {  # check <label> <expected-substring> <<< haystack is $3
  CELLS=$((CELLS + 1))
  if printf '%s' "$3" | grep -qF -- "$2"; then
    echo "  ok    $1"
  else
    echo "  FAIL  $1"
    echo "        expected to find: $2"
    FAILED=$((FAILED + 1))
  fi
}

git rev-parse --verify --quiet "$BASE^{commit}" >/dev/null || { echo "SKIP: $BASE not in this clone"; exit 0; }
git rev-parse --verify --quiet "$TIP^{commit}"  >/dev/null || { echo "SKIP: $TIP not in this clone";  exit 0; }

out=$(scripts/push-consent.sh "$BASE" "$TIP" 2>&1)

echo "range $BASE..$TIP"
check "counts the range"                 "commits      23"                                   "$out"
check "names the Rust-covered half"      "13 of 23 commit(s) touch Rust"                     "$out"
check "names the half it cannot cover"   "10 of 23 commit(s) touch NO .rs file"              "$out"
check "lists a no-Rust commit by sha"    "8a701877"                                          "$out"
check "warns against quoting a Rust gate" "Do not offer a green Rust gate"                   "$out"
check "reports a reachability section"   "commit(s),"                                        "$out"

# An empty range must say so and exit clean, not fall through the report.
empty=$(scripts/push-consent.sh "$TIP" "$TIP" 2>&1)
check "empty range says nothing to push" "Nothing to push"                                   "$empty"
check "empty range reports zero"         "commits      0"                                    "$empty"

echo
if [ "$FAILED" -eq 0 ]; then
  echo "PASS ($CELLS outcome cells)"
  exit 0
fi
echo "FAIL ($FAILED of $CELLS outcome cells)"
exit 1
