#!/bin/bash
# Launch one playwright-mcp lane instance, for use as the ExecStart of a
# systemd template unit (amux-playwright-mcp@.service.template). The
# instance name (systemd's %i) is "<lane>-<port>", e.g. "frontstage-8931" —
# split here rather than passed as two separate unit parameters because
# systemd template units only carry one %i.
#
# `exec` replaces this shell with the npx/node process so systemd tracks
# the real MCP server as the service's main PID (required for
# Type=simple + Restart=always to behave correctly — otherwise systemd
# tracks this wrapper shell, which exits immediately while node lingers,
# and a crash of the real process would not trigger a restart).
set -euo pipefail

INSTANCE="${1:?usage: amux-playwright-mcp.sh <lane>-<port>}"
LANE="${INSTANCE%-*}"
PORT="${INSTANCE##*-}"

PROFILE="$HOME/.amux/playwright-profile"
if [ "$LANE" != "frontstage" ]; then
  PROFILE="${PROFILE}-${LANE}"
fi

export DISPLAY="${DISPLAY:-:0}"
export NO_UPDATE_NOTIFIER=1

exec npx -y @playwright/mcp@latest \
  --port "$PORT" \
  --user-data-dir "$PROFILE" \
  --host 0.0.0.0
