#!/usr/bin/env bash
# Cells for the AF-195 test receipt: does the pre-commit report actually
# discriminate, or does it print a reassuring line no matter what?
#
# Every cell drives the REAL hook block, extracted by line range from the
# shipped file rather than paraphrased, because a check pinning a copy of the
# logic is exactly as green as one pinning the wrong layer (ethos rule 7).
set -u
ROOT_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK="$ROOT_REPO/scripts/git-hooks/pre-commit"
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok() { if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "  ok   $1"; else
       fail=$((fail+1)); echo "  FAIL $1: want [$3] got [$2]"; fi; }

# The shipped block, from its banner to EOF. Extracted, never retyped.
START=$(grep -n 'DOES YOUR GREEN RESULT DESCRIBE THIS COMMIT' "$HOOK" | cut -d: -f1)
[ -n "$START" ] || { echo "FATAL: the receipt block is gone from $HOOK"; exit 1; }
sed -n "${START},\$p" "$HOOK" > "$TMP/block.sh"

run() {  # run(receipt_file, staged_paths) -> stdout
  # A FRESH home per cell. Sharing one let cell c's receipt survive into cell
  # d's "no receipt" case, which passed for the wrong reason on the first run.
  ( export AMUX_HOME="$TMP/home.$RANDOM$RANDOM" AMUX_SESSION=cell
    mkdir -p "$AMUX_HOME/test-receipts"
    [ -n "$1" ] && cp "$1" "$AMUX_HOME/test-receipts/cell.tsv"
    cd "$ROOT_REPO" || exit
    ROOT="$ROOT_REPO"; STAGED="$2"
    # shellcheck disable=SC1090
    . "$TMP/block.sh" ) 2>&1
}

# A real staged path and its real index sha, so the comparison is against git.
REALP=$(git -C "$ROOT_REPO" ls-files 'crates/*.rs' | head -1)
REALSHA=$(git -C "$ROOT_REPO" ls-files -s -- "$REALP" | awk '{print $2}')
mkr() {  # mkr(rc, sha) -> receipt path
  f="$TMP/r$RANDOM.tsv"
  { printf '# repo\t%s\n' "$ROOT_REPO"
    printf '# head\tdeadbeef\n'
    printf '# rc\t%s\n' "$1"
    printf '# at\t%s\n' "$(date -u +%s)"
    printf '# args\t-p amux-server --lib\n'
    printf '%s\t%s\n' "$2" "$REALP"; } > "$f"
  echo "$f"
}

echo "cell a: staged bytes match the tested bytes -> says so"
o=$(run "$(mkr 0 "$REALSHA")" "$REALP")
ok "reports a match" "$(echo "$o" | grep -c 'match the bytes')" "1"
ok "does not cry change" "$(echo "$o" | grep -c 'DIFFER')" "0"

echo "cell b: THE POSITIVE CONTROL — staged bytes moved since the run"
o=$(run "$(mkr 0 0000000000000000000000000000000000000000)" "$REALP")
ok "reports the drift" "$(echo "$o" | grep -c 'DIFFER')" "1"
ok "names the file" "$(echo "$o" | grep -c "$REALP")" "1"
ok "withholds the reassurance" "$(echo "$o" | grep -c 'match the bytes')" "0"

echo "cell c: the last run was RED — it must not vouch for anything"
o=$(run "$(mkr 101 "$REALSHA")" "$REALP")
ok "says the run failed" "$(echo "$o" | grep -c 'EXITED 101')" "1"
ok "no green claim on a red run" "$(echo "$o" | grep -c 'match the bytes')" "0"

echo "cell d: no receipt at all — silence must not read as coverage"
o=$(run "" "$REALP")
ok "says there is none" "$(echo "$o" | grep -c 'none for session')" "1"

echo "cell e: a receipt from another checkout claims nothing about this one"
f=$(mkr 0 "$REALSHA")
grep -v '^# repo' "$f" > "$f.x"; { printf '# repo\t/somewhere/else\n'; cat "$f.x"; } > "$f"
o=$(run "$f" "$REALP")
ok "names the other checkout" "$(echo "$o" | grep -c 'DIFFERENT checkout')" "1"

echo "cell f: no staged crate files — the block stays silent"
o=$(run "$(mkr 0 "$REALSHA")" "README.md")
ok "silent on a non-crate commit" "$(echo "$o" | grep -c 'test receipt')" "0"

echo "cell g: a staged file the run never saw is called out, not passed"
o=$(run "$(mkr 0 "$REALSHA")" "$REALP
crates/amux-server/src/never_tested_xyz.rs")
ok "flags the unseen path" "$(echo "$o" | grep -c 'not in the tested set')" "1"

echo ""
echo "test-test-receipt: $pass passed, $fail failed"
[ "$fail" = 0 ]
