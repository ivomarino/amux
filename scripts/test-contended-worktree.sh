#!/usr/bin/env bash
# The contention wrapper must report the WORKTREE, on both arms (AF-356).
#
# WHY. The wrapper measured one of the two ways a peer reddens your suite. It
# sampled the auto-builder's lock and printed "NOT build contention", which reads
# as "therefore your bug" — while the other cause, a peer's UNCOMMITTED SOURCE in
# the shared worktree, produces the identical symptom: a red in a module you never
# opened that passes on rerun.
#
# Measured live 2026-08-31: `amux` got one failure in gate_table_matches_python,
# read the clean verdict, concluded ETXTBSY, and carried that into a verification
# request as its stated weakest evidence line. The real cause was a peer's
# uncommitted ItemType::Decision sitting in the tree.
#
# BOTH ARMS is the load-bearing part. A caveat inside one branch is one the other
# branch's reader never sees (ethos rule 1), and a dirty tree explains a red just
# as well when the builder WAS running.
#
# `cargo` is stubbed via PATH so no real suite runs; the wrapper's own git calls
# run against a throwaway repo, never this checkout.
set -uo pipefail
cd "$(dirname "$0")/.."
WRAP="$(pwd)/scripts/test-contended.sh"
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1"; }
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin"
cat > "$TMP/bin/cargo" <<'STUB'
#!/usr/bin/env bash
echo "test result: ok. 1 passed; 0 failed"
exit 0
STUB
chmod +x "$TMP/bin/cargo"

# A throwaway git repo, so `git status` inside the wrapper describes THIS fixture.
REPO="$TMP/repo"; mkdir -p "$REPO"
( cd "$REPO" && git init -q && git config user.email t@t && git config user.name t \
  && echo base > tracked.txt && git add tracked.txt && git commit -qm base ) >/dev/null 2>&1

run_wrap() { # $1 = lock dir to fake (empty string = no builder)
  ( cd "$REPO" && PATH="$TMP/bin:$PATH" \
      AMUX_RS_BUILD_LOCK="${1:-$TMP/nonexistent-lock}" \
      CARGO_TARGET_DIR="$TMP/target" \
      bash "$WRAP" -p whatever 2>&1 )
}

echo "== contended worktree cells =="

# (a) CLEAN TREE must SAY it is clean. Silence would be indistinguishable from
#     "this clause did not run", which is the exact absent-vs-measured confusion
#     the wrapper exists to fix one level down.
out=$(run_wrap "")
if printf '%s' "$out" | grep -q "worktree:  clean at start and end"; then ok "(a) a clean tree is stated, not left silent"
else bad "(a) a clean tree must be stated"; printf '%s\n' "$out" | sed 's/^/       /'; fi

# (b) DIRTY TREE is reported, and the FILE IS NAMED. A count alone would not let
#     a reader tell in one second whether the path is theirs.
echo changed > "$REPO/tracked.txt"
out=$(run_wrap "")
if printf '%s' "$out" | grep -q "uncommitted file(s) at start"; then ok "(b) a dirty tree is reported"
else bad "(b) a dirty tree must be reported"; printf '%s\n' "$out" | sed 's/^/       /'; fi
if printf '%s' "$out" | grep -q "worktree:    tracked.txt"; then ok "(b2) and the dirty file is NAMED"
else bad "(b2) the dirty file must be named, not just counted"; fi

# (c) BOTH ARMS. This is the cell the whole file is for: with the builder lock
#     present the wrapper takes the ETXTBSY branch, and the worktree clause must
#     STILL print. Without this, moving the clause inside the else-branch — the
#     obvious way to write it — passes every other cell here.
LOCK="$TMP/lock"; mkdir -p "$LOCK"; echo 4242 > "$LOCK/pid"
out=$(run_wrap "$LOCK")
if printf '%s' "$out" | grep -q "A BUILD WAS IN FLIGHT"; then ok "(c) the builder arm was actually taken"
else bad "(c) fixture did not reach the builder arm, so (c2) proves nothing"; fi
if printf '%s' "$out" | grep -q "uncommitted file(s) at start"; then ok "(c2) the worktree clause prints on the BUILDER arm too"
else bad "(c2) the worktree clause is missing on the builder arm"; printf '%s\n' "$out" | sed 's/^/       /'; fi

# (d) NO OWNER IS INVENTED. Guessing an owner from mtime has been wrong on this
#     checkout repeatedly (AF-179), and a confident wrong owner is worse than a
#     named file with none. Pin that the disclaimer travels with the list.
if printf '%s' "$out" | grep -q "Owner is NOT inferred here"; then ok "(d) the report declines to guess an owner, and says so"
else bad "(d) the no-owner disclaimer must travel with the file list"; fi

# (e) EXIT STATUS IS THE TEST COMMAND'S, untouched. The wrapper reports, it never
#     decides, and the new clause must not have changed that.
( cd "$REPO" && PATH="$TMP/bin:$PATH" AMUX_RS_BUILD_LOCK="$TMP/none" \
    CARGO_TARGET_DIR="$TMP/target" bash "$WRAP" -p whatever ) >/dev/null 2>&1
[ $? -eq 0 ] && ok "(e) a passing stub still exits 0 through the wrapper" \
              || bad "(e) the wrapper altered the exit status"

echo
echo "  $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
