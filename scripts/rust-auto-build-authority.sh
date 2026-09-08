#!/usr/bin/env bash
# The launchd builder must not execute whichever revision happens to be checked
# out in the shared developer tree. A second checkout can be locally ahead (or
# simply stale) while still pointing its timer at this path; that was the ATE-93
# takeover loop. This launcher is the single normal activation authority: it
# runs the committed builder only from a clean detached worktree at origin/main.
set -euo pipefail

REPO="${AMUX_AUTHORITY_REPO:-}"
REF="${AMUX_RS_ACTIVATION_REF:-origin/main}"
WORK="${AMUX_AUTHORITY_WORKTREE:-$HOME/.amux/activation-source}"
MARKER="${AMUX_AUTHORITY_MARKER:-${WORK}.authority}"
LOG="${AMUX_AUTHORITY_LOG:-$HOME/.amux/logs/rust-auto-build.log}"
mkdir -p "$(dirname "$LOG")"

say() { echo "== !! ACTIVATION AUTHORITY $*" >> "$LOG"; }
refuse() { say "REFUSED — $*"; exit 0; }

[ -n "$REPO" ] || refuse "AMUX_AUTHORITY_REPO is unset; no checkout may choose the live image"
target=$(git -C "$REPO" rev-parse --verify -q "${REF}^{commit}" 2>/dev/null) \
  || refuse "cannot resolve $REF in $REPO; refusing an unmeasured activation"

if [ -e "$WORK" ]; then
  [ -f "$MARKER" ] || refuse "$WORK exists without this launcher's marker"
  [ "$(cat "$MARKER")" = "$REPO" ] || refuse "$WORK marker names a different source repository"
  git -C "$WORK" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
    || refuse "$WORK is not a Git worktree"
  [ -z "$(git -C "$WORK" status --porcelain --untracked-files=all)" ] \
    || refuse "$WORK is dirty; preserving it instead of activating unknown bytes"
  git -C "$WORK" checkout --detach -q "$target" \
    || refuse "could not advance clean authority worktree to $target"
else
  mkdir -p "$(dirname "$WORK")"
  git -C "$REPO" worktree add --detach "$WORK" "$target" >/dev/null \
    || refuse "could not create authority worktree at $WORK"
  printf '%s\n' "$REPO" > "$MARKER"
fi

say "BUILD $target from detached $REF worktree=$WORK (source checkout=$REPO)"
AMUX_REPO="$WORK" AMUX_RS_ACTIVATION_REF="$REF" "$WORK/scripts/rust-auto-build.sh"
