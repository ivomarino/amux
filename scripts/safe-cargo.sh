#!/usr/bin/env bash
# Run cargo in its OWN systemd scope, isolated from whatever pane invoked it.
#
# Root cause this exists for (AMUX-70, frustrations.md 2026-09-01, confirmed
# live via journalctl + dmesg): every process in an interactive amux pane —
# including the Claude Code session itself — shares ONE systemd scope,
# `tmux-spawn-<uuid>.scope`. When a `cargo check`/`clippy`/`build`/`test` run
# directly in that pane gets OOM-killed, systemd does not just reap the
# offending process — it marks the WHOLE SCOPE `Failed with result
# 'oom-kill'`, and whatever supervises the pane tears it down and starts a
# brand-new one. The entire interactive session restarts mid-conversation,
# not just the build.
#
# `systemd-run --user --scope` gives the cargo invocation a SIBLING scope
# instead (verified: `run-p<pid>-i<id>.scope`, distinct from
# `tmux-spawn-*.scope`) — an OOM kill inside it can no longer cascade into
# the pane hosting the session.
#
# This does NOT replace remote offload (see CLAUDE.md / the offload-builds
# convention) — always prefer building on remote hardware for anything
# beyond a quick syntax check. Use this script only for the cases that
# genuinely need to run locally, so a local run is contained instead of
# risky by default.
#
# Usage: scripts/safe-cargo.sh <cargo subcommand and args...>
#   scripts/safe-cargo.sh check -p amux-server
#   scripts/safe-cargo.sh clippy -p amux-server --all-targets -- -D warnings
set -euo pipefail

if ! command -v systemd-run >/dev/null 2>&1; then
  echo "safe-cargo.sh: systemd-run not found — refusing to run cargo unisolated." \
       "Offload remotely instead, or run systemd-run --user --scope by hand." >&2
  exit 1
fi

exec systemd-run --user --scope --quiet \
  --working-directory="$(pwd)" \
  --setenv=CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.amux/rust-build-target}" \
  --setenv=PATH="$PATH" \
  --setenv=HOME="$HOME" \
  -- cargo "$@"
