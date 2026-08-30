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

# --browser only accepts chrome/firefox/webkit/msedge — none of those match
# this box's actual browser (system chromium at /usr/bin/chromium, no
# Google Chrome build installed under any of the channels). Point directly
# at the real binary instead; falls through to Playwright's own bundled
# chromium if the system one is ever removed, rather than hard-failing.
CHROMIUM_BIN="/usr/bin/chromium"
EXEC_ARGS=()
[ -x "$CHROMIUM_BIN" ] && EXEC_ARGS=(--executable-path "$CHROMIUM_BIN")

# This box has a link-local-only IPv6 address on eth0 (no default route, no
# global address) — enough for Cloudflare's Turnstile client-side capability
# check to conclude the browser "has" IPv6 and route it to an AAAA-only
# challenge host, which is then genuinely unreachable (confirmed 2026-08-28:
# brunhild.challenges.cloudflare.com has no A record at all). --disable-ipv6
# makes Chromium stop reporting IPv6 capability entirely, so Cloudflare falls
# back to an IPv4-reachable challenge host like it would for a real
# IPv4-only visitor. Passed via --config since --disable-ipv6 is a Chromium
# flag, not a playwright-mcp CLI flag.
CONFIG_FILE="$HOME/.amux/playwright-mcp-config.json"
CONFIG_ARGS=()
[ -f "$CONFIG_FILE" ] && CONFIG_ARGS=(--config "$CONFIG_FILE")

exec npx -y @playwright/mcp@latest \
  --port "$PORT" \
  --user-data-dir "$PROFILE" \
  --host 0.0.0.0 \
  "${EXEC_ARGS[@]}" \
  "${CONFIG_ARGS[@]}"
