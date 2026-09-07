#!/usr/bin/env bash
# Who must you ask before `git push origin main`, and who CANNOT be asked?
#
# CLAUDE.md's Deploy block says "If foreign commits exist, ask their author
# before pushing." That instruction has no truthful path when the author is an
# ISOLATED lane: an isolated worker refuses sends carrying a worker origin, so
# only the owner can reach it from the dashboard. The state is completely
# legitimate — an isolated lane is supposed to commit to shared main and
# somebody else is supposed to push it — and the two available moves are to
# push unasked while a MANDATORY rule says otherwise, or to never push. Everyone
# picks the first and nobody records that they did (AF-548, found live
# 2026-09-07 when amux-homepage's consent poll named 16 of 23 commits and missed
# 5 belonging to an isolated `amux`).
#
# A NAMED EXEMPTION IS A TRUTHFUL PATH; SILENCE IS NOT (ethos rule 1: when you
# exempt something from a loop, name what still reaches it). So this prints the
# unaskable authors as their own section, by name and with the reason, for the
# pusher to state rather than not know.
#
# It also prints, per lane, how many commits touch NO Rust — because the gates
# people offer as push evidence (`cargo clippy --workspace`, `cargo test -p
# amux-server`) are scoped to a LANGUAGE and get quoted as a verdict on a RANGE.
# In the run that produced this script, 10 of 23 commits touched no .rs file at
# all, including the commit belonging to the lane that asked for consent. A
# workspace clippy said nothing whatsoever about it, and "exit 0" looks total.
#
# Usage: scripts/push-consent.sh [<base>] [<tip>]     defaults: origin/main main
set -uo pipefail

BASE="${1:-origin/main}"
TIP="${2:-main}"
ME="${AMUX_SESSION:-}"
API="${AMUX_URL:-}"

git rev-parse --verify --quiet "$BASE" >/dev/null || { echo "no such ref: $BASE" >&2; exit 2; }
git rev-parse --verify --quiet "$TIP"  >/dev/null || { echo "no such ref: $TIP"  >&2; exit 2; }

shas=$(git rev-list "$BASE..$TIP")
total=$(printf '%s\n' "$shas" | grep -c . || true)

echo "range        $BASE..$TIP"
echo "commits      $total"
echo "you          ${ME:-<AMUX_SESSION unset>}"
echo

if [ "$total" -eq 0 ]; then
  echo "Nothing to push."
  exit 0
fi

# ---- per-commit facts -------------------------------------------------------
# lane<TAB>sha<TAB>rust_files<TAB>total_files
facts=""
for c in $shas; do
  lane=$(git log -1 --format='%(trailers:key=Amux-Session,valueonly,separator=)' "$c")
  [ -z "$lane" ] && lane="<no Amux-Session trailer>"
  files=$(git show --name-only --format= "$c")
  nrs=$(printf '%s\n' "$files" | grep -c '\.rs$' || true)
  ntot=$(printf '%s\n' "$files" | grep -c . || true)
  facts="${facts}${lane}	$(git rev-parse --short "$c")	${nrs}	${ntot}
"
done

lanes=$(printf '%s' "$facts" | cut -f1 | sort -u)

# ---- reachability -----------------------------------------------------------
# An UNKNOWN reachability is reported as unknown, never as reachable: a probe
# that could not run must not read as a clean answer (ethos rule 4).
lane_state() {  # -> "isolated" | "reachable" | "unknown:<why>"
  local lane="$1"
  [ -z "$API" ] && { echo "unknown:AMUX_URL unset"; return; }
  local body
  body=$(curl -sk --max-time 5 "$API/api/sessions/$lane" 2>/dev/null) || { echo "unknown:session lookup failed"; return; }
  [ -z "$body" ] && { echo "unknown:empty response"; return; }
  printf '%s' "$body" | python3 -c '
import json,sys
try: d=json.load(sys.stdin)
except Exception: print("unknown:unparseable response"); sys.exit()
if not isinstance(d,dict) or "name" not in d: print("unknown:no such session"); sys.exit()
if "isolated" not in d: print("unknown:payload has no isolated field"); sys.exit()
print("isolated" if d["isolated"] else "reachable")
'
}

ask=""; cannot=""; unknown=""; mine=""
for lane in $lanes; do
  n=$(printf '%s' "$facts" | awk -F'\t' -v l="$lane" '$1==l' | grep -c . || true)
  nors=$(printf '%s' "$facts" | awk -F'\t' -v l="$lane" '$1==l && $3==0' | grep -c . || true)
  row="  $lane — $n commit(s), $nors with no Rust"
  if [ -n "$ME" ] && [ "$lane" = "$ME" ]; then mine="${mine}${row}
"; continue; fi
  state=$(lane_state "$lane")
  case "$state" in
    isolated)   cannot="${cannot}${row}   [isolated: refuses worker-origin sends]
" ;;
    reachable)  ask="${ask}${row}
" ;;
    unknown:*)  unknown="${unknown}${row}   [${state#unknown:}]
" ;;
  esac
done

[ -n "$mine" ]   && { echo "YOURS (no consent needed):"; printf '%s' "$mine"; echo; }
[ -n "$ask" ]    && { echo "MUST ASK — reachable, send them 'amux send <lane>':"; printf '%s' "$ask"; echo; }
[ -n "$cannot" ] && {
  echo "CANNOT BE ASKED — state this in your push, do not imply you asked:"
  printf '%s' "$cannot"
  echo "  An isolated lane committing to shared main has already accepted that a peer"
  echo "  will push it. Pushing is defensible; claiming consent is not. (AF-548)"
  echo
}
[ -n "$unknown" ] && { echo "REACHABILITY UNKNOWN — treat as must-ask, not as clear:"; printf '%s' "$unknown"; echo; }

# ---- what the Rust gates cannot speak for -----------------------------------
# Computed, never written: the test of a summary line is what input would change
# it. A constant has none, so it reads as measured while being unable to fail.
no_rust=$(printf '%s' "$facts" | awk -F'\t' '$3==0' | grep -c . || true)
with_rust=$((total - no_rust))
echo "GATE SCOPE:"
echo "  $with_rust of $total commit(s) touch Rust — 'cargo clippy --workspace' / 'cargo test -p amux-server' speak for these."
echo "  $no_rust of $total commit(s) touch NO .rs file — those gates say nothing about them."
if [ "$no_rust" -gt 0 ]; then
  echo "  Do not offer a green Rust gate as push-readiness for these:"
  printf '%s' "$facts" | awk -F'\t' '$3==0 {printf "    %s [%s] %s file(s), 0 Rust\n", $2, $1, $4}'
fi
