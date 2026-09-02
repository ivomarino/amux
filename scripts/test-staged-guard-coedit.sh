#!/usr/bin/env bash
# Cells for the co-edit warning's mtime CORROBORATION (AF-391).
#
# WHY THIS EXISTS. The warning says a peer may have uncommitted work in a file
# you are about to commit whole. It is mtime-derived and says so, and on a
# checkout ~125 lanes share that makes the BUSIEST lane the default suspect for
# any file whose mtime moves. mixpeek-general was named as the editor of a file
# they had never opened; clearing it cost them and mixpeek-cicd a verification
# each. Their first check was the one wired here: worktree bytes already
# committed, so there is no uncommitted content to be in dispute.
#
# THE PROPERTY UNDER TEST IS THE ASYMMETRY, not the quiet. Downgrade only when
# the claim is mtime-derived AND nothing is in dispute; a transcript-backed
# co-edit, or a file with real uncommitted content, must still print in full.
# A test that only checked "it got quieter" would pass a guard that had been
# hollowed out, which is the failure this whole file family exists to prevent.
#
# Runs the SHIPPED loop body against a real git fixture, not a paraphrase.
set -uo pipefail
cd "$(dirname "$0")/.."
HOOK="${STAGED_GUARD_HOOK:-$(pwd)/scripts/git-hooks/amux-staged-guard}"
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1"; echo "       got: $2"; }

TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=init.defaultBranch GIT_CONFIG_VALUE_0=main

echo "== staged-guard co-edit corroboration cells =="

# A repo with two files: one whose worktree bytes are committed (nothing in
# dispute) and one carrying real uncommitted content.
git init -q "$TMP/r"
( cd "$TMP/r"
  printf 'committed\n' > settled.txt
  printf 'committed\n' > dirty.txt
  git add settled.txt dirty.txt; git commit -qm base
  printf 'uncommitted edit\n' >> dirty.txt ) >/dev/null 2>&1

run_case() { # $1 = python dict for one shared-file entry
  python3 - "$HOOK" "$TMP/r" "$1" <<'PY' 2>&1
import io, os, subprocess, sys, ast
src = open(sys.argv[1]).read()
os.chdir(sys.argv[2])
# Take the SHIPPED helper and the SHIPPED loop body, never a retyped copy.
h0 = src.index("def _nothing_in_dispute(path):")
h1 = src.index("\n\n", src.index("        return None, \"the check errored\""))
helper = src[h0:h1]
b0 = src.index('    for f in (d.get("shared") or []):')
b1 = src.index("    # PRE-COMMIT MISATTRIBUTES A PEER'S WRITE TO AN INNOCENT HOOK")
body = "\n".join(l[4:] if l.startswith("    ") else l for l in src[b0:b1].splitlines())
buf = io.StringIO()
g = {"subprocess": subprocess, "w": buf.write, "d": {"shared": [ast.literal_eval(sys.argv[3])]}}
exec(helper, g)
exec(body, g)
sys.stdout.write(buf.getvalue())
PY
}

# (a) THE REPORTED CASE: mtime-derived claim on a file with nothing in dispute.
out=$(run_case "{'path':'settled.txt','owner':'peer','peer':True,'age_secs':600,'has_unstaged_changes':False,'mine_provenance':'observed','their_provenance':'transcript'}")
if printf '%s' "$out" | grep -q "NOT corroborated"; then ok "(a) an uncorroborated mtime claim on settled content is downgraded"
else bad "(a) expected the downgrade line" "$out"; fi
if printf '%s' "$out" | grep -q "settled.txt"; then ok "(a) the file is still NAMED, so nothing actionable disappears"
else bad "(a) the downgraded line must still name the file" "$out"; fi
if printf '%s' "$out" | grep -q "git apply --cached"; then bad "(a) the eight-line remedy must not print for a dispute that does not exist" "$out"
else ok "(a) the remedy block is gone, which is the cost being removed"; fi

# (b) CONTROL, the one that stops this being a hollowing-out: REAL uncommitted
#     content still prints the full warning even though the claim is mtime-derived.
out=$(run_case "{'path':'dirty.txt','owner':'peer','peer':True,'age_secs':600,'has_unstaged_changes':True,'mine_provenance':'observed','their_provenance':'transcript'}")
if printf '%s' "$out" | grep -q "git apply --cached"; then ok "(b) a file with real uncommitted content still gets the full warning"
else bad "(b) the full warning must survive when content IS in dispute" "$out"; fi
if printf '%s' "$out" | grep -q "NOT corroborated"; then bad "(b) must not downgrade a file that has uncommitted content" "$out"
else ok "(b) no downgrade when there is something to dispute"; fi

# (c) CONTROL: a transcript-backed claim on BOTH sides is not the weak claim and
#     must never be downgraded, even on settled content.
out=$(run_case "{'path':'settled.txt','owner':'peer','peer':True,'age_secs':600,'has_unstaged_changes':False,'mine_provenance':'transcript','their_provenance':'transcript'}")
if printf '%s' "$out" | grep -q "NOT corroborated"; then bad "(c) a two-sided transcript claim is not mtime-derived and must stand" "$out"
else ok "(c) only an mtime-derived claim is eligible for downgrade"; fi

# (d) An unknown path: the check cannot run, and that must not read as settled.
out=$(run_case "{'path':'no-such-file.txt','owner':'peer','peer':True,'age_secs':600,'has_unstaged_changes':True,'mine_provenance':'observed','their_provenance':'transcript'}")
if printf '%s' "$out" | grep -q "NOT corroborated"; then bad "(d) an unrunnable check must not be reported as 'nothing in dispute'" "$out"
else ok "(d) a check that could not run leaves the warning standing"; fi

echo
echo "  $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
