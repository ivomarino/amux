#!/usr/bin/env bash
# AF-243 — the RETIREMENT tool had no test, and it is the one that DELETES.
#
# scripts/frustrations-archive.py removes an entry from frustrations.md and writes it
# to frustrations-archive.md. That is the only sanctioned way an entry leaves the
# ledger, it is destructive to the source file, and it shipped with zero coverage —
# then gained a whole new code path (`--superseded`, the third disposition) on
# 2026-08-26, still with none.
#
# THE PROPERTY THAT MATTERS is not "the entry ended up in the archive". It is that the
# ledger loses EXACTLY the target entry and nothing else. A tool that took one extra
# entry, or truncated the tail, would satisfy every naive assertion — the target is in
# the archive, the target is gone from the ledger — while silently destroying a
# neighbour. So the cells below pin the SURVIVORS, by name, on both sides.
#
# THE CARD WRITE IS UNREACHABLE HERE, ON PURPOSE. `carry_to_card` is best-effort by
# design: the archive is what makes a move recoverable, so a failed card write must
# never block the move or leave an entry half-retired. Pointing it at a closed port is
# therefore not a limitation of this harness, it is the cell that proves the asymmetry
# holds — the entry must still move, and the failure must be REPORTED rather than
# swallowed.
#
# Exit 0 = all pass, 1 = a failure.
set -uo pipefail
cd "$(dirname "$0")/.."
# Overridable so a MUTANT runs through the same cells (ethos rule 7).
ARCHIVE_TOOL="${FRUSTRATIONS_ARCHIVE:-$(pwd)/scripts/frustrations-archive.py}"
PASS=0; FAIL=0
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

ok()   { PASS=$((PASS+1)); printf '  ok   — %s\n' "$1"; }
bad()  { FAIL=$((FAIL+1)); printf '  FAIL — %s\n' "$1"; }
want() { [ "$2" = "$3" ] && ok "$1" || { bad "$1"; printf '         want %s got %s\n' "$3" "$2"; }; }
has()  { grep -qF "$2" "$3" && ok "$1" || bad "$1 (missing: $2)"; }
lacks(){ grep -qF "$2" "$3" && bad "$1 (present but must not be: $2)" || ok "$1"; }

# A throwaway repo: scripts/ + a ledger with THREE entries, so "removed exactly one"
# is a claim with survivors on both sides of the target.
build() { # dir
  local d="$1"; mkdir -p "$d/scripts"
  cp "$ARCHIVE_TOOL" "$d/scripts/frustrations-archive.py"
  cat > "$d/frustrations.md" <<'LEDGER'
# amux frustrations

Header prose. The template below is indented so it cannot count itself.

```
  ## <title>
  AREA: <area>
```

---
## FIRST entry, must survive
AREA: cli
SEVERITY: annoys
STATUS: open
DATE: 2026-08-01
SESSION: lane-a
CARD: X-1
SYMPTOM: first symptom text
COST: first cost text

## TARGET entry, the one being retired
AREA: gates
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-02
SESSION: lane-b
CARD: X-2
SYMPTOM: target symptom text
COST: target cost text

## LAST entry, must survive
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-03
SESSION: lane-c
CARD: X-3
SYMPTOM: last symptom text
COST: last cost text
LEDGER
}

run() { # dir args...
  local d="$1"; shift
  ( cd "$d" && AMUX_URL="https://127.0.0.1:9" python3 scripts/frustrations-archive.py "$@" ) \
    > "$d/out.txt" 2>&1
  echo $?
}

echo "AF-243 — frustrations retirement tool"

# ---- (a) --list names every entry and mutates nothing -----------------------
A="$TMP/a"; build "$A"
before=$(shasum -a 256 "$A/frustrations.md" | cut -d' ' -f1)
rc=$(run "$A" --list)
want "(a) --list exits 0" "$rc" 0
has  "(a) --list names the target" "TARGET entry" "$A/out.txt"
has  "(a) --list names a survivor" "FIRST entry" "$A/out.txt"
after=$(shasum -a 256 "$A/frustrations.md" | cut -d' ' -f1)
want "(a) --list does not touch the ledger" "$after" "$before"

# ---- (b) VALIDATED: exactly one entry moves ---------------------------------
B="$TMP/b"; build "$B"
LN=$(cd "$B" && python3 scripts/frustrations-archive.py --list | grep -F "TARGET entry" | awk '{print $1}' | tr -d 'L')
rc=$(run "$B" "$LN" lane-b --evidence-stdin <<< "the evidence line")
want "(b) exits 0" "$rc" 0
lacks "(b) the target LEFT the ledger"        "TARGET entry" "$B/frustrations.md"
has   "(b) the entry BEFORE it survived"      "FIRST entry"  "$B/frustrations.md"
has   "(b) the entry AFTER it survived"       "LAST entry"   "$B/frustrations.md"
has   "(b) the header survived"               "Header prose" "$B/frustrations.md"
has   "(b) it landed in the archive"          "TARGET entry" "$B/frustrations-archive.md"
has   "(b) stamped VALIDATED with the signer" "VALIDATED: lane-b | the evidence line" "$B/frustrations-archive.md"
has   "(b) the entry body came with it"       "target symptom text" "$B/frustrations-archive.md"
lacks "(b) survivors did NOT follow it"       "FIRST entry" "$B/frustrations-archive.md"

# ---- (c) the card write is unreachable, and that must not block the move ----
has  "(c) the unreachable card write is REPORTED, not swallowed" "NOT carried" "$B/out.txt"

# ---- (d) --superseded stamps differently ------------------------------------
D="$TMP/d"; build "$D"
LN=$(cd "$D" && python3 scripts/frustrations-archive.py --list | grep -F "TARGET entry" | awk '{print $1}' | tr -d 'L')
rc=$(run "$D" "$LN" lane-b --superseded --evidence-stdin <<< "the mechanism was wrong")
want  "(d) exits 0" "$rc" 0
has   "(d) stamped SUPERSEDED" "SUPERSEDED: lane-b | the mechanism was wrong" "$D/frustrations-archive.md"
# ANCHORED to line start on purpose. A whole-file grep for "VALIDATED:" matches the
# archive HEADER, which explains the VALIDATED: line in prose — so the first version of
# this cell went red against correct code. A stamp is a line, and only a line.
n=$(grep -c '^VALIDATED:' "$D/frustrations-archive.md" || true)
want "(d) NO entry is stamped VALIDATED — the whole point" "$n" 0
has   "(d) says so on stdout"  "SUPERSEDED (entry was WRONG)" "$D/out.txt"
lacks "(d) the target still left the ledger" "TARGET entry" "$D/frustrations.md"
has   "(d) survivors intact"   "LAST entry" "$D/frustrations.md"

# ---- (e) a bad line refuses and changes nothing -----------------------------
E="$TMP/e"; build "$E"
before=$(shasum -a 256 "$E/frustrations.md" | cut -d' ' -f1)
rc=$(run "$E" 999999 lane-b --evidence-stdin <<< "x")
want "(e) a line with no entry exits 1" "$rc" 1
after=$(shasum -a 256 "$E/frustrations.md" | cut -d' ' -f1)
want "(e) and the ledger is untouched" "$after" "$before"
[ -n "$before" ] && ok "(e) the hash is non-empty, so that comparison could have failed" \
                 || bad "(e) hash was empty — the check could not fail"

printf '\n  %d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
