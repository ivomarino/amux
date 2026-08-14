#!/usr/bin/env bash
# Build and run amux-server for the e2e harness from COMMITTED HEAD (AMUX-2924).
#
# WHY NOT THE WORKING TREE. This is a SHARED checkout — CLAUDE.md's deploy
# section is about several sessions committing to it at once — and playwright's
# webServer used to run `cargo run -p amux-server` right here in it. So any peer
# who happened to be mid-edit when your suite started failed YOUR run, with a
# Rust compile error, against changes that might be JS-only. That happened on
# 2026-08-11: E0432 out of session_verbs.rs during a run whose diff touched only
# app.js and a .spec.ts. `cargo check` passed 40s later, once the peer saved.
#
# The lost minutes are not the point. The danger is that a red e2e run caused by
# a stranger's half-saved file is INDISTINGUISHABLE from a red run caused by
# your own change, so the natural response — doubt your patch — is aimed at the
# wrong thing. Committed source is also what actually ships (the builder deploys
# HEAD, not the tree), so HEAD is the honest thing to test.
#
# THE TRAP THIS COULD HAVE ADDED, and why the notice below is not decoration:
# defaulting to HEAD means uncommitted work is NOT under test. A developer
# iterating on a fix would watch their change have no effect, which is a
# quieter failure than the one being fixed. So a dirty tree is announced
# loudly, by name, every run — and AMUX_E2E_WORKING_TREE=1 opts back in for
# exactly that workflow.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# One shared target dir, per CLAUDE.md: per-session scratch dirs cost 10-15GB
# each and filled the volume on 2026-08-10. Cargo's build lock makes a
# concurrent build WAIT and then find the work done, which is cheap.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.amux/rust-build-target}"

dirty="$(git -C "$REPO" status --porcelain -- crates/ Cargo.toml Cargo.lock 2>/dev/null || true)"

if [ "${AMUX_E2E_WORKING_TREE:-0}" = "1" ]; then
  echo "[e2e] AMUX_E2E_WORKING_TREE=1 — building the WORKING TREE, not HEAD."
  echo "[e2e] A peer mid-edit in this shared checkout can fail this run; that is the trade you asked for."
  exec cargo run -p amux-server
fi

if [ -n "$dirty" ]; then
  echo "[e2e] ─────────────────────────────────────────────────────────────────"
  echo "[e2e] BUILDING FROM COMMITTED HEAD ($(git -C "$REPO" rev-parse --short HEAD))."
  echo "[e2e] These UNCOMMITTED rust changes are NOT under test:"
  echo "$dirty" | sed 's/^/[e2e]   /'
  echo "[e2e] Commit them, or re-run with AMUX_E2E_WORKING_TREE=1 to test the tree."
  echo "[e2e] ─────────────────────────────────────────────────────────────────"
fi

# A STABLE worktree, re-pointed at HEAD each run rather than created per run:
# `worktree add` costs real time and a per-run temp dir leaks when playwright
# kills this process (it kills the server, it does not run our cleanup).
WT="${AMUX_E2E_WORKTREE_DIR:-$HOME/.amux/e2e-worktree}"
SHA="$(git -C "$REPO" rev-parse HEAD 2>/dev/null || true)"

# FALL BACK TO THE TREE RATHER THAN FAILING TO START. `set -e` plus a git that
# cannot make a worktree — a shallow CI checkout, a read-only $HOME, a git too
# old — would otherwise kill this script, and playwright reports that as
# "Process from config.webServer was not able to start", which reads as a
# broken harness rather than a git problem. On CI the tree IS HEAD, so the
# fallback is exact there; locally it is the old behaviour with a loud line
# saying so. Degrading to the previous behaviour beats refusing to run.
setup_worktree() {
  [ -n "$SHA" ] || return 1
  if [ ! -e "$WT/.git" ]; then
    rm -rf "$WT"
    git -C "$REPO" worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || return 1
  else
    # Already exists: move it to this HEAD. Concurrent e2e runs on this
    # checkout share ONE HEAD, so they converge on the same sha rather than
    # fighting over the directory.
    git -C "$WT" checkout --detach "$SHA" >/dev/null 2>&1 || {
      git -C "$REPO" worktree remove --force "$WT" >/dev/null 2>&1 || rm -rf "$WT"
      git -C "$REPO" worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || return 1
    }
  fi
  [ -f "$WT/Cargo.toml" ]
}

if setup_worktree; then
  cd "$WT"
  # The worktree build gets its OWN target dir (AMUX-2961). Sharing the fleet's
  # dir looked like the rule — but cargo dep-info records ABSOLUTE source paths,
  # so after a worktree build, a `cargo build` from the repo compares mtimes of
  # the WORKTREE's files, sees them unchanged, and no-ops in 0.13s — silently
  # handing back HEAD's binary while the caller believes they built their tree.
  # That manufactured three consecutive "my fix doesn't work" verdicts on
  # 2026-08-12, and would make the auto-builder install a STALE binary for the
  # next commit while reporting success. One extra bounded cache dir is the
  # price of the two source trees never sharing fingerprints.
  export CARGO_TARGET_DIR="${AMUX_E2E_HEAD_TARGET_DIR:-$HOME/.amux/rust-build-target-e2e-head}"
else
  echo "[e2e] WARNING: could not prepare a HEAD worktree at $WT — falling back to the WORKING TREE."
  echo "[e2e] On CI the tree is HEAD so this is exact; on a shared checkout a peer mid-edit can fail this run."
fi

exec cargo run -p amux-server
