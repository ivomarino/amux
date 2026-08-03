#!/usr/bin/env bash
# Catch the amux server IN THE ACT of spinning and dump its stack (AC-170).
#
# The spin is intermittent and self-clearing: the process pegs a core, /api/board
# and /api/email/* hang, then launchd replaces it. By the time a human notices,
# the evidence is a fresh pid. Three wedges produced the SHAPE every time and the
# cause never, for exactly this reason.
#
# So this watches rather than samples. /health gained store/store_ms/degraded in
# AC-164 precisely so the degradation is observable from outside; this is the
# consumer that makes that signal worth having. (An earlier version of this comment
# claimed a well-timed dump would NAME the spinning function. It does not, for a
# 10-thread process — see the correction below, which is why the evidence capture
# now leads with ps -M rather than with stacks.)
#
# Uses SIGUSR1, NOT py-spy. py-spy REQUIRES ROOT ON macOS, so on this machine it
# is not an available instrument for an unattended session — discovered by
# running it, not by reading about it. The server registers faulthandler on
# SIGUSR1 instead: it dumps ITS OWN stacks into ~/.amux/logs/server.log, needs no
# privileges, and does not exit. Verified against a synthetic pure-Python spin —
# the looping frame is named with its line number.
#
# ── WHAT 625 CAPTURED EVENTS TAUGHT ME, and it corrected this script's premise ──
# The header above used to claim "STAT=R means the stack will NAME the function".
# That is FALSE for a 10-thread server, and believing it nearly produced a wrong
# root cause. Two things went wrong, both in this file:
#
# 1. faulthandler dumps EVERY thread and ranks NOTHING. Nine of ten threads sit in
#    time.sleep() or select() at any instant, so the dump names them all and the
#    hot one is not marked. The first frames I read (_watch_self:63742,
#    _watch_server_env:63601, _build_proc_info:928, _scheduler_loop:12425) are ALL
#    literally `time.sleep(...)` lines. A stack dump answers "where is each thread",
#    never "which thread is burning CPU" — so it cannot, alone, find a spin.
#    THE FIX: capture `ps -M <pid>` WITH every dump. It gives per-thread %CPU and
#    STAT, which is the discriminator faulthandler structurally cannot express.
#
# 2. `tail -c 4000` kept the WRONG END. A full dump is ~10.5KB, so the cap silently
#    discarded the front of it — and the threads it cut were the informative ones
#    while the idle sleepers it kept looked like a plausible answer. An evidence cap
#    that biases toward a wrong conclusion is worse than no evidence.
#    THE FIX: capture the whole dump, delimited, no cap.
#
# And the finding those fixes produced: there is NO spinning thread. Per-thread CPU
# was 38.5 / 14.3 / 2.6 / ~0 with every STAT=S, and total accumulated CPU was ~13s
# over 442s of uptime (~3% average) against instantaneous reads of 80-89%. The
# process is BURSTY, not spinning. A >=70% trigger on a bursty process-wide gauge is
# why this fired 625 times: those are not 625 wedges, they are 625 normal bursts.
# Read the trigger below with that in mind — CPU alone does not indicate the fault.
#
# Read-only in the sense that matters: SIGUSR1 only makes the process print. It
# does not stop or restart anything — the standing rule on this machine's
# launchd agents holds.
#
#   ./scripts/spin-catcher.sh [seconds_between_polls] [cpu_trigger] [sustain_polls]
set -uo pipefail
AMUX="${AMUX_URL:-https://localhost:8822}"
POLL="${1:-3}"
CPU_TRIP="${2:-0}"        # 0 = OFF. See below: this server idles at ~102%.
SUSTAIN="${3:-5}"          # polls of sustained CPU before believing the gauge
OUT="${HOME}/.amux/spin-dumps"
mkdir -p "$OUT"

echo "spin-catcher: polling ${AMUX}/health every ${POLL}s"
# The banner must describe the ACTUAL trigger. It previously advertised "store != ok"
# and a CPU threshold after both had been narrowed — a watcher that misreports its own
# trigger is the same class of defect as one that cannot fire.
echo "  trigger: /health unanswered  OR  store in (hung|error|unavailable)  OR  degraded"
if [ "$CPU_TRIP" = "0" ]; then
  echo "  cpu trigger: OFF (this server's normal state is ~102%; it cannot discriminate)"
else
  echo "  cpu trigger: >= ${CPU_TRIP}% for ${SUSTAIN} consecutive polls (explicitly enabled)"
fi
echo "  evidence -> ${OUT}/evidence-*.txt (ps -M per-thread CPU + WHOLE faulthandler dumps)"

caught=0
HOT=0                      # consecutive polls over CPU_TRIP; `set -u` is on, so this
                           # must exist before the first read or the loop dies at poll 1
while :; do
  H="$(curl -sk --max-time 4 "${AMUX}/health" 2>/dev/null)"
  if [ -z "$H" ]; then
    # /health itself not answering is ALSO the event: the old endpoint could not
    # express this, and a silent curl failure is how it stayed invisible before.
    TS="$(date -u +%Y%m%dT%H%M%SZ)"
    echo "[$TS] /health did not answer — server may be gone or fully wedged" | tee -a "$OUT/events.log"
    PID="$(pgrep -f 'amux-server\.py' | head -1)"
    if [ -n "$PID" ]; then
      echo "[$TS] pid $PID still alive with /health dead — dumping" | tee -a "$OUT/events.log"
      kill -USR1 "$PID" 2>/dev/null && echo "  -> SIGUSR1 sent; stacks appended to ~/.amux/logs/server.log" 
      caught=$((caught+1))
    fi
    sleep "$POLL"; continue
  fi
  read -r PID CPU STORE MS DEG <<<"$(printf '%s' "$H" | python3 -c '
import json,sys
d=json.load(sys.stdin)
print(d.get("pid",0), d.get("cpu_percent",0), d.get("store","?"),
      d.get("store_ms",-1), 1 if d.get("degraded") else 0)
' 2>/dev/null)"
  [ -z "${PID:-}" ] && { sleep "$POLL"; continue; }

  TRIP=0; WHY=""
  # store/degraded are the TRUSTWORTHY triggers: they are the symptom users
  # actually report (board and email hanging) and across 625 captured events they
  # never once fired. That is a real result — the wedge is not a store stall.
  # Only `hung` — NOT `slow`. Baseline sampling caught store=slow/store_ms=260 during
  # ordinary operation, so tripping on any non-ok simply re-imports the false-positive
  # problem through a different field. The live wedge read store=hung, store_ms=800.
  case "$STORE" in hung|error|unavailable) TRIP=1; WHY="store=$STORE" ;; esac
  [ "$DEG" = "1" ] && { TRIP=1; WHY="${WHY} degraded"; }

  # CPU is the UNTRUSTWORTHY one and must not trip alone. /health's cpu_percent is
  # a process-wide gauge over a 0.5s window across 10 threads, so ordinary bursts
  # read 80-89% while the true average is ~3%. Tripping on that produced 625
  # "events" and zero findings — a detector that fires constantly reports nothing,
  # and it costs a SIGUSR1 to the live server plus a 10KB log write every time.
  # SUSTAINED is the discriminator a single sample cannot express: require the
  # high reading to persist across consecutive polls before believing it.
  # CPU trigger is OFF unless a threshold is passed explicitly. Measured baseline on
  # this machine: a steady 102.5% with store=ok and store_ms=0-44. A threshold below
  # that describes NORMAL OPERATION, so it cannot discriminate a fault — sustain only
  # cut 625 trips to 53. Each trip costs the live server two SIGUSR1 stack dumps and
  # ~20KB written into server.log, i.e. the instrument was adding load to the exact
  # log-lock contention it had just diagnosed (AC-174). A detector that perturbs the
  # system it measures, while never having produced a true positive, should be off.
  if [ "$CPU_TRIP" != "0" ] && awk "BEGIN{exit !($CPU >= $CPU_TRIP)}"; then
    HOT=$((HOT+1))
    [ "$HOT" -ge "${SUSTAIN:-5}" ] && { TRIP=1; WHY="${WHY} cpu>=${CPU_TRIP} for ${HOT} polls"; }
  else
    HOT=0
  fi

  if [ "$TRIP" = "1" ]; then
    TS="$(date -u +%Y%m%dT%H%M%SZ)"
    echo "[$TS] TRIP pid=$PID cpu=$CPU store=$STORE store_ms=$MS degraded=$DEG why:${WHY}" | tee -a "$OUT/events.log"
    D="$OUT/evidence-${TS}-pid${PID}.txt"
    # PER-THREAD CPU FIRST, and this is the frame-of-reference the stacks lack:
    # ps -M ranks threads by %CPU and shows STAT. A single thread at ~100% with
    # STAT=R is a real spin; ten threads at 5% each is ordinary load that the
    # process-wide gauge merely presents as alarming. Captured BEFORE the dumps so
    # it describes the same moment that tripped.
    { echo "=== ps -M (per-thread CPU — the discriminator faulthandler cannot express) ==="
      ps -M "$PID" 2>&1
      echo; echo "=== process totals ==="; ps -o pid=,%cpu=,etime=,rss= -p "$PID" 2>&1
      echo; echo "=== /health at trip ==="; printf '%s\n' "$H"
    } > "$D" 2>&1
    # Mark the log so the dump can be sliced out whole. The previous `tail -c 4000`
    # cut a ~10.5KB dump to its last 4KB and kept the idle threads while discarding
    # the working ones — evidence truncation that actively favoured a wrong answer.
    before=$(wc -c < "$HOME/.amux/logs/server.log" 2>/dev/null || echo 0)
    kill -USR1 "$PID" 2>/dev/null
    sleep 2
    kill -USR1 "$PID" 2>/dev/null
    sleep 1
    { echo; echo "=== faulthandler stacks, WHOLE (both dumps, uncapped) ==="
      dd if="$HOME/.amux/logs/server.log" bs=1 skip="$before" 2>/dev/null
    } >> "$D"
    caught=$((caught+1))
    echo "[$TS] evidence -> $D (total events: $caught)" | tee -a "$OUT/events.log"
    HOT=0
    sleep 20   # one event, not a dump storm
  fi
  sleep "$POLL"
done
