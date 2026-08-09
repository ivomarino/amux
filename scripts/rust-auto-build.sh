#!/usr/bin/env bash
# Auto-build for the Rust server (the "server adopts every change" seam).
#
# Run by com.amux.server-rs-builder every 60s: when the committed Rust
# source has moved since the last successful build, rebuild release and
# install the binary; the running server notices its own binary changed and
# exits for launchd to relaunch (self-adoption in amux-server/src/lib.rs).
#
# COMMITTED source only — building the working tree would ship half-typed
# code from any session on this shared checkout. A commit is the unit of
# "there is a change to adopt", mirroring how the Python server's file-save
# reload is bounded by whole-file saves.
set -euo pipefail

REPO="/Users/ethan/Dev/amux"
INSTALL="$HOME/.local/bin/amux-server-rs"
STAMP="$HOME/.amux/rust-build-stamp"
LOG="$HOME/.amux/logs/rust-auto-build.log"
mkdir -p "$(dirname "$LOG")" "$(dirname "$INSTALL")"

head=$(git -C "$REPO" log -1 --format=%H -- crates/ Cargo.toml Cargo.lock 2>/dev/null || echo none)
last=$(cat "$STAMP" 2>/dev/null || echo "")
[ "$head" = "$last" ] && exit 0

{
  echo "== $(date '+%F %T') building $head (was: ${last:-none})"
  # Build from a clean, committed snapshot: a worktree of HEAD, so nobody's
  # uncommitted edits (or a mid-edit broken tree) can poison the deploy.
  WORK=$(mktemp -d /tmp/amux-rs-build.XXXXXX)
  trap 'git -C "$REPO" worktree remove --force "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT
  git -C "$REPO" worktree add --detach "$WORK" "$(git -C "$REPO" rev-parse HEAD)" >/dev/null
  # Shared target dir: incremental rebuilds (~15s) instead of cold ones
  # (~3min) — the worktree isolates SOURCE, the cache is content-keyed.
  if (cd "$WORK" && CARGO_TARGET_DIR="$HOME/.amux/rust-build-target" cargo build --release -p amux-server 2>&1 | tail -3); then
    install -m 0755 "$HOME/.amux/rust-build-target/release/amux-server" "$INSTALL"
    echo "$head" > "$STAMP"
    echo "== installed; running server will self-adopt within 5s"
  else
    echo "== BUILD FAILED for $head — running server keeps the last good build"
    # Stamp is NOT updated: the next cycle retries. A failed build never
    # takes the fleet down (the AC-309 class: a bad save must not crash-loop
    # the server).
  fi
} >> "$LOG" 2>&1
