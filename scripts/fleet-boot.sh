#!/usr/bin/env bash
# amux fleet cold-start — bring every non-archived worker back up after a reboot.
#
# WHY THIS EXISTS
#
# On 2026-08-29 the machine restarted and 56 of the fleet's 58 non-archived
# workers stayed down until a human noticed and started them by hand. Nothing was
# broken in the sense of an error being raised: nothing had ever been responsible
# for this. launchd brought back the four amux SERVICES it knows about
# (com.amux.server-rs, its builder, the watchdog, cert-renew) and the watchdog
# supervises the server only, by design and with an explicit rationale. The
# WORKERS — the 56 processes that are the actual point of the fleet — had no
# owner at boot at all. The dashboard showed them registered, described, holding
# 69 cards in `doing`, and not running.
#
# That is the gap this file closes. It is deliberately the smallest thing that
# can close it: wait for the server, then call the one bulk-start verb, then say
# what happened.
#
# WHY IT WAITS FOR THE SERVER
#
# A worker's whole value is that it can reach the API — its board cards, its
# memory, its identity. `amux start` injects AMUX_URL into every worker it
# spawns, resolving it through `amux url`, and workers whose provider is
# codex/ollama/gemini have their launch DELEGATED to the server outright. Racing
# the server means those workers launch against a base that is not answering yet.
# So we wait, with a bounded timeout, and we start the fleet either way — a
# claude-provider worker that comes up before the server recovers on its own is
# better than a fleet that stayed down because a health check was slow.
#
# WHY NOT KeepAlive
#
# This is a cold-start, not a supervisor. Restarting a worker that a human
# deliberately stopped would be amux deciding something that is the human's to
# decide (ethos rule 8). It runs once, at login, and exits.

set -uo pipefail

AMUX_BIN="${AMUX_BIN:-/Users/ethan/Dev/amux/amux}"
LOG="${AMUX_FLEET_BOOT_LOG:-$HOME/.amux/logs/fleet-boot.log}"
HEALTH_TIMEOUT="${AMUX_FLEET_BOOT_HEALTH_TIMEOUT:-120}"

mkdir -p "$(dirname "$LOG")"

log() { printf '%s %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*" >> "$LOG"; }

log "=== fleet-boot starting (uptime: $(uptime | sed 's/^ *//')) ==="

if [[ ! -x "$AMUX_BIN" ]]; then
  log "FATAL: amux CLI not executable at $AMUX_BIN — fleet NOT started"
  exit 1
fi

# Resolve the API base the same way the CLI does, so a port change cannot strand
# this script the way a hardcoded 8822 stranded so much else.
# AMUX_FLEET_BOOT_BASE exists so the server-down path can actually be exercised
# (ethos rule 7). Point it at a dead port and the WARN branch must fire; if it
# does not, this loop is not a check. Verified 2026-08-29 against a closed port.
base="${AMUX_FLEET_BOOT_BASE:-$("$AMUX_BIN" url 2>/dev/null || echo "https://localhost:8824")}"

# Wait for /health. Report which way it ended — "waited and gave up" and "came up
# in 3s" produce identical fleet outcomes on the happy path and completely
# different ones when a worker fails, so the log has to distinguish them.
# The endpoint is `/health`, NOT `/api/health` — the latter is a 404. This
# mattered because the first version of this loop tested `curl -sk ... >/dev/null`
# and read CURL'S EXIT STATUS, which is 0 for a perfectly-delivered 404. So it
# declared the server up, instantly, against an endpoint that does not exist, and
# would have declared it up just as fast with the server stopped. That is the
# amux-wide "read the BODY, never the exit code" rule (see cmd_fresh) in the one
# place where believing it costs the whole fleet: a false "up" here is what sends
# codex/ollama/gemini workers at a server that is not answering.
#
# So: check the HTTP CODE, and check that the body actually says status ok.
health_tmp="$(mktemp -t amux-fleet-boot-health)"
waited=0
server_up=0
while (( waited < HEALTH_TIMEOUT )); do
  code="$(curl -sk --max-time 5 "$base/health" -o "$health_tmp" -w '%{http_code}' 2>/dev/null || true)"
  if [[ "$code" == "200" ]] && grep -q '"status":"ok"' "$health_tmp" 2>/dev/null; then
    server_up=1
    break
  fi
  sleep 3
  waited=$((waited + 3))
done
rm -f "$health_tmp"

if (( server_up == 1 )); then
  log "server answered $base/health with status ok after ${waited}s"
else
  log "WARN: server did NOT return a healthy $base/health within ${HEALTH_TIMEOUT}s — starting the fleet anyway; codex/ollama/gemini workers may fail to launch"
fi

# The stagger matters more here than anywhere else: this runs while launchd is
# still bringing the rest of the machine up.
export AMUX_START_ALL_STAGGER="${AMUX_START_ALL_STAGGER:-2}"

summary="$("$AMUX_BIN" start-all 2>&1)"
rc=$?

# Strip ANSI so the log is greppable.
printf '%s\n' "$summary" | sed 's/\x1b\[[0-9;]*m//g' >> "$LOG"
log "start-all exited rc=$rc"

# Independent verdict. `start-all`'s own count is what IT believes it did; this
# is what the server can actually see, which is the number that matters and the
# one that would have exposed the original silent-abort bug immediately.
if (( server_up == 1 )); then
  # The payload goes to a FILE and python reads the file by name.
  #
  # Two quoting/plumbing traps were hit writing this, both of which produced a
  # verdict line that looked like a measured negative:
  #   * `python3 -c '...'` whose script contained an inner `'` — the shell closed
  #     the string early and the probe emitted nothing.
  #   * `curl ... | python3 - <<'EOF'` — the heredoc IS stdin, so it overrode the
  #     pipe and `json.load(sys.stdin)` got an empty read. curl was healthy the
  #     whole time (200, 173KB, 0.1s); only the plumbing was wrong.
  # Reading a named file has neither edge, and a missing/short file is a
  # distinguishable, reportable state rather than a silent empty parse.
  sess_tmp="$(mktemp -t amux-fleet-boot)"
  http="$(curl -sk --max-time 45 "$base/api/sessions" -o "$sess_tmp" -w '%{http_code}' 2>/dev/null)"
  if [[ "$http" != "200" ]]; then
    verdict="VERDICT UNAVAILABLE: /api/sessions returned HTTP ${http:-<none>}"
  else
    verdict="$(SESS_FILE="$sess_tmp" python3 <<'PYEOF' 2>&1
import json, os
try:
    with open(os.environ["SESS_FILE"]) as fh:
        d = json.load(fh)
except Exception as e:
    print("VERDICT UNAVAILABLE: could not parse /api/sessions (%s)" % e)
    raise SystemExit(0)
live = [x for x in d if not x.get("archived")]
run = [x for x in live if x.get("running")]
down = sorted(x["name"] for x in live if not x.get("running"))
tail = (" · still down: " + ", ".join(down)) if down else " · all up"
print("verdict: %d/%d non-archived workers running%s" % (len(run), len(live), tail))
PYEOF
)"
  fi
  rm -f "$sess_tmp"
  log "${verdict:-VERDICT UNAVAILABLE: /api/sessions probe produced nothing}"
else
  log "verdict skipped — server never answered, so 'still down' could not be distinguished from 'unreadable'"
fi

log "=== fleet-boot done ==="
exit "$rc"
