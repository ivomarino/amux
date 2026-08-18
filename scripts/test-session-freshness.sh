#!/usr/bin/env bash
# AEAB-18 — the SessionStart freshness hook must warn when THIS checkout has
# diverged, because an append to a shared append-only file here reaches nobody.
#
# The failure being guarded is silent and write-shaped. A merely-stale checkout
# announces itself the moment you pull. A DIVERGED one does not: appending to
# frustrations.md succeeds, prints nothing, and never reaches origin, because the
# hourly sync job refuses to fast-forward (correctly — it must not rewrite a
# shared tree). On 2026-08-17 four entries went in that way; that copy held 25
# entries while origin held 124, and it was noticed by chance days later.
#
# Every case builds REAL git repos and runs the SHIPPED hook as a subprocess, so
# this exercises the actual dispatch path rather than a paraphrase of its logic.
#
# The (a) and (c)/(d) cases are the load-bearing ones: a hook that warned
# unconditionally would satisfy (b) while being pure noise, and noise is how a
# banner gets ignored — which the hook's own header calls out as the reason it
# stays silent when everything is current.
#
# Exit 0 = all pass, 1 = a failure. Wired into .github/workflows/checks.yml.
set -uo pipefail
cd "$(dirname "$0")/.."
HOOK="$(pwd)/.claude/session-freshness.sh"
PASS=0; FAIL=0
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
# Pin the default branch instead of inheriting it. The first cut of this file did
# not, and passed locally (init.defaultBranch=main) while failing 4 assertions in
# CI, where git has no default set and falls back to `master`: the bare repo's
# HEAD was master, the pushes created main, and the clone ended up tracking
# nothing — so the hook found no upstream and printed NOTHING. The failure then
# read as "the hook is broken" when the harness was.
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=init.defaultBranch GIT_CONFIG_VALUE_0=main

# Build: a bare origin, a clone, and the hook installed inside the clone.
# `n_behind` commits land on origin after the clone; `n_ahead` land locally.
mk() { # $1 name  $2 n_ahead  $3 n_behind
  local d="$TMP/$1"
  git init -q --bare -b main "$d/origin.git"
  git clone -q "$d/origin.git" "$d/work" 2>/dev/null
  ( cd "$d/work"
    git checkout -q -B main
    mkdir -p .claude
    cp "$HOOK" .claude/session-freshness.sh
    echo seed > seed.txt; git add -A; git commit -qm seed
    git push -q -u origin main
  )
  if [ "$3" -gt 0 ]; then   # commits that exist ONLY on origin
    git clone -q "$d/origin.git" "$d/other" 2>/dev/null
    ( cd "$d/other"; git checkout -q -B main origin/main
      for i in $(seq 1 "$3"); do echo "up$i" > "up$i.txt"; git add -A; git commit -qm "up$i"; done
      git push -q origin main )
  fi
  if [ "$2" -gt 0 ]; then   # commits that exist ONLY here
    ( cd "$d/work"; for i in $(seq 1 "$2"); do echo "loc$i" > "loc$i.txt"; git add -A; git commit -qm "loc$i"; done )
  fi
  # HARNESS SELF-CHECK, before the hook is consulted at all. Without it a setup
  # bug is indistinguishable from a hook bug — which is exactly how the CI
  # failure above read. If the repo is not in the shape the case name claims,
  # say SETUP and fail loudly rather than blaming the thing under test.
  ( cd "$d/work"
    git fetch -q origin 2>/dev/null
    a=$(git rev-list --count origin/main..HEAD 2>/dev/null || echo x)
    b=$(git rev-list --count HEAD..origin/main 2>/dev/null || echo x)
    if [ "$a" != "$2" ] || [ "$b" != "$3" ]; then
      echo "SETUP BROKEN for '$1': wanted ahead=$2 behind=$3, got ahead=$a behind=$b"
      exit 0
    fi
    bash .claude/session-freshness.sh 2>&1
  )
}

# A harness failure is not a test result. Surface it as its own failure so it can
# never be mistaken for the hook misbehaving.
setup_ok() { case "$2" in *"SETUP BROKEN"*) FAIL=$((FAIL+1)); echo "$2" | head -1; return 1;; esac; return 0; }
says()  { case "$2" in *"$1"*) PASS=$((PASS+1));; *) FAIL=$((FAIL+1)); echo "FAIL: expected output to mention '$1'"; echo "  got: ${2:-<empty>}";; esac; }
lacks() { case "$2" in *"$1"*) FAIL=$((FAIL+1)); echo "FAIL: output should NOT mention '$1'"; echo "  got: ${2:-<empty>}";; *) PASS=$((PASS+1));; esac; }

MARK="do NOT append to frustrations.md here"

# (a) CONTROL — current checkout: the hook must stay SILENT. Without this, a hook
#     that printed the warning unconditionally would pass every other case.
out=$(mk clean 0 0)
if [ -z "$(printf '%s' "$out" | tr -d '[:space:]')" ]; then PASS=$((PASS+1));
else FAIL=$((FAIL+1)); echo "FAIL: (a) a current checkout must produce NO output; got: $out"; fi

# (b) THE INCIDENT — diverged (unpushed AND behind): must warn, and must say why.
out=$(mk diverged 2 3)
says "DIVERGED" "$out"
says "$MARK" "$out"
says "2 unpushed" "$out"
lacks "reached nobody" "$out"   # the rationale belongs in the rule, not the banner

# (c) Behind ONLY — recoverable by a pull, so the strand warning must NOT fire.
#     It should still report the ordinary staleness line.
out=$(mk behind 0 3)
lacks "$MARK" "$out"
lacks "DIVERGED" "$out"
says "commit(s) behind" "$out"

# (d) Ahead ONLY — ordinary in-flight work; a push still reaches origin.
out=$(mk ahead 2 0)
lacks "$MARK" "$out"
lacks "DIVERGED" "$out"

echo
echo "test-session-freshness: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
