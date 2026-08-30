#!/usr/bin/env bash
# Cells for the observed-edits walk root (AMUX-3920, handed over from MHC-527).
#
# THE DEFECT. The harness reports the SESSION cwd and the hook walked exactly
# that. When cwd is a SUBDIRECTORY of a shared checkout, every edit above it
# produced no record at all. MHC-527's control, one Bash call writing two files
# in one repo: `homepage/scripts/.probe` recorded, `scripts/.probe` not, hook
# logged n=1. In that session 13 of 16 committed paths were above cwd — 81%
# structurally unobservable — and the staged guard, correctly reading "NO
# session has an edit record for this", asked for VERIFIED_SOLO on three commits
# and ALLOW_FOREIGN on two. Overriding a guard that is right most of the time is
# how a fleet learns to wave it through.
#
# THE CONSTRAINT THAT SHAPES THE FIX. amux is 2,222 files and walks in 0.03s;
# ~/Dev/mixpeek is ~640,000 and sits right on the 1.5s budget — 2.5-2.9s COLD,
# ~1.0s once the filesystem cache is warm. (My first figure was 2.54s and I
# reported it without noting it was a cold walk; alternating the arms three times
# is what separated the two.) On a box that compiles continuously the cold case
# is not rare, and mixpeek-homepage-claude confirms every recent run on their
# lane carries TRUNCATED=budget.
#
# So a walk that starts at the repo root can exhaust the budget before ever
# reaching the session's own directory — trading a known blind spot for an
# unpredictable one.
#
# Hence: cwd first, then the rest of the repo with the remaining budget, and a
# distinct marker when the budget or cap cuts. Never less coverage than before,
# more when it fits, and the shortfall is named rather than silent.
set -uo pipefail
cd "$(dirname "$0")/.."
HOOK="${OBSERVED_EDITS_HOOK:-$(pwd)/scripts/claude-hooks/observed-edits-post.py}"
PASS=0; FAIL=0
ok(){ PASS=$((PASS+1)); printf '  ok   %s\n' "$1"; }
no(){ FAIL=$((FAIL+1)); printf '  FAIL %s\n     %s\n' "$1" "${2:-}"; }

# Run the hook against a scratch repo whose session cwd is a SUBDIRECTORY.
# Echoes the last log line. $1 = python file to run (allows a patched copy).
run_probe() {
  local hookfile="$1" tmp
  tmp="$(mktemp -d)"
  local R="$tmp/repo"
  mkdir -p "$R/sub/deep" "$R/top"
  git init -q "$R"
  export AMUX_HOME="$tmp/home" AMUX_SESSION="probe-3920"
  mkdir -p "$AMUX_HOME/hooks/state"
  # The PRE half writes this marker; its mtime is t0 and must precede the writes.
  touch "$AMUX_HOME/hooks/state/observed-$AMUX_SESSION.t0"
  sleep 1
  echo a > "$R/top/above.txt"
  echo b > "$R/sub/deep/below.txt"
  # `cp` rather than `echo`: a pure-read command claims nothing (AF-124) and the
  # hook correctly returns before walking.
  printf '{"cwd":"%s","tool_input":{"command":"cp src dst"}}' "$R/sub" \
    | AMUX_URL="http://127.0.0.1:9" python3 "$hookfile" >/dev/null 2>&1
  tail -1 "$AMUX_HOME/hooks/state/observed-edits.log" 2>/dev/null || echo ""
  rm -rf "$tmp"
}

echo "observed-edits walk cells (AMUX-3920)"

line="$(run_probe "$HOOK")"
case "$line" in
  *top/above.txt*) ok "a file ABOVE the session cwd is recorded" ;;
  *) no "the above-cwd file is still invisible — the whole defect" "$line" ;;
esac
# CONTROL: widening must not lose what already worked.
case "$line" in
  *sub/deep/below.txt*) ok "the file below cwd is still recorded" ;;
  *) no "coverage that worked before must not regress" "$line" ;;
esac
# The paths sent to the server are absolute; the LOG is repo-relative. Once the
# walk root moved above cwd this line rendered every hit as ../../../.., which
# for an observability hook is most of the defect.
case "$line" in
  *../../*) no "log paths must be repo-relative, not ../../.." "$line" ;;
  *) ok "log paths are repo-relative and readable" ;;
esac
# n= must not double-count. The second root CONTAINS the first, and on macOS
# /var vs /private/var makes the same file two strings — found by an n=3 in a
# two-file probe.
case "$line" in
  *"n=2 "*) ok "n= counts each file once across both roots" ;;
  *) no "n= should be exactly 2 for a two-file probe" "$line" ;;
esac

# TRUNCATION IS NAMED, and cwd coverage survives it. On the monorepo above this
# is the EXPECTED path, so "found 3" and "found 3 so far" must not read alike.
CAP="$(mktemp -d)/cap.py"
sed 's/^MAX_PATHS = 80/MAX_PATHS = 1/' "$HOOK" > "$CAP"
line="$(run_probe "$CAP")"
case "$line" in
  *TRUNCATED=cap*) ok "a capped walk says so" ;;
  *) no "a truncated walk must be distinguishable from a clean one" "$line" ;;
esac
# THE ORDERING GUARANTEE: under a cut budget the session's OWN directory is the
# part that survives, because it is walked first. Without this the fix would
# trade a known blind spot for an unpredictable one.
case "$line" in
  *sub/deep/below.txt*) ok "under truncation the surviving path is the cwd one" ;;
  *) no "cwd must be walked FIRST so its coverage is never the part that is lost" "$line" ;;
esac

# A CACHE WRITE IS NOT A LANE'S EDIT (mixpeek-cicd's specimen: n=3 in which two
# of the three recorded paths were .pytest_cache and .ruff_cache entries). Those
# become edit records, and an edit record is what the staged guard reads to
# decide who touched a file — so a test run minted attribution for files no
# guard should care about. `.cache`-prefixed names were already excluded;
# `.pytest_cache` is not `.cache`-prefixed, which is how it slipped through.
cache_probe() {
  local tmp; tmp="$(mktemp -d)"
  local R="$tmp/repo"
  mkdir -p "$R/sub" "$R/.pytest_cache/v" "$R/.ruff_cache/x"
  git init -q "$R"
  export AMUX_HOME="$tmp/home" AMUX_SESSION="probe-cache"
  mkdir -p "$AMUX_HOME/hooks/state"
  touch "$AMUX_HOME/hooks/state/observed-$AMUX_SESSION.t0"
  sleep 1
  echo real > "$R/sub/real.txt"
  echo junk > "$R/.pytest_cache/v/cache.json"
  echo junk > "$R/.ruff_cache/x/entry"
  printf '{"cwd":"%s","tool_input":{"command":"cp src dst"}}' "$R/sub" \
    | AMUX_URL="http://127.0.0.1:9" python3 "$HOOK" >/dev/null 2>&1
  tail -1 "$AMUX_HOME/hooks/state/observed-edits.log" 2>/dev/null || echo ""
  rm -rf "$tmp"
}
line="$(cache_probe)"
case "$line" in
  *pytest_cache*|*ruff_cache*) no "a tool cache write must not become an edit record" "$line" ;;
  *) ok "tool-cache writes are pruned, not attributed" ;;
esac
# CONTROL: pruning caches must not lose the real file beside them.
case "$line" in
  *sub/real.txt*) ok "the real edit beside the caches is still recorded" ;;
  *) no "pruning must not drop genuine edits" "$line" ;;
esac

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
