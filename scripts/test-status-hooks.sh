#!/usr/bin/env bash
# Canonical status-hook installation and payload regression cells.
set -euo pipefail
cd "$(dirname "$0")/.."
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
SETTINGS="$TMP/settings.json"

printf '%s\n' '{
  "model": "keep-me",
  "hooks": {
    "Stop": [{"hooks": [
      {"type":"command","command":"echo unrelated"},
      {"type":"command","command":"curl $AMUX_URL/api/sessions/$AMUX_SESSION/report"}
    ]}],
    "PostToolUse": [{"matcher":"Write","hooks":[
      {"type":"command","command":"bash check-format.sh"}
    ]}]
  }
}' > "$SETTINGS"

for _ in 1 2; do
  /usr/bin/python3 scripts/hooks/install-claude-status-hooks.py \
    --settings "$SETTINGS" --hook-path '$HOME/.amux/hook-report.sh' >/dev/null
done

/usr/bin/python3 - "$SETTINGS" <<'PY'
import json, sys
v=json.load(open(sys.argv[1]))
assert v["model"] == "keep-me"
hooks=v["hooks"]
required={"SessionStart","UserPromptSubmit","PostToolUse","Stop","SubagentStart","SubagentStop"}
assert required <= set(hooks)
rows=[]
for event, groups in hooks.items():
    for group in groups:
        for hook in group.get("hooks", []):
            rows.append((event, group.get("matcher"), hook.get("command", "")))
reports=[r for r in rows if "hook-report.sh" in r[2]]
assert len(reports) == 6, reports
assert len([r for r in reports if r[0] == "PostToolUse" and r[1] == ".*"]) == 1
assert any(r[2] == "echo unrelated" for r in rows)
assert any(r[2] == "bash check-format.sh" for r in rows)
assert not any("/api/sessions/" in r[2] and "hook-report.sh" not in r[2] for r in rows)
print("ok   installer is idempotent and preserves unrelated hooks/settings")
PY

# Drive the shipped hook against a disposable HTTP endpoint. The endpoint can
# refuse requests or drop a response after reading the body, which exercises
# the durable producer rather than a paraphrase of its JSON encoder.
mkdir -p "$TMP/home/.amux/logs"
CAPTURE="$TMP/requests.jsonl" PORT_FILE="$TMP/port" DOWN="$TMP/down" LOSS="$TMP/loss"
cat > "$TMP/server.py" <<'PY'
import http.server,json,os,socketserver,sys
capture,port_file,down,loss=sys.argv[1:]
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get("content-length","0")))
        try: body=json.loads(raw)
        except Exception: body={"invalid":raw.decode(errors="replace")}
        row={"body":body,"down":os.path.exists(down),"path":self.path}
        with open(capture,"a") as stream:
            stream.write(json.dumps(row,separators=(",",":"))+"\n"); stream.flush()
        if os.path.exists(down):
            self.send_response(503); self.end_headers(); return
        if body.get("agent_id")=="poison-agent":
            self.send_response(400); self.end_headers(); return
        if os.path.exists(loss):
            os.unlink(loss)
            self.connection.shutdown(2)
            self.connection.close()
            return
        self.send_response(200); self.send_header("content-type","application/json")
        self.end_headers(); self.wfile.write(b'{"ok":true}')
    def log_message(self,*args): pass
class Server(socketserver.TCPServer): allow_reuse_address=True
with Server(("127.0.0.1",0),Handler) as server:
    with open(port_file,"w") as stream: stream.write(str(server.server_address[1]))
    server.serve_forever()
PY
/usr/bin/python3 "$TMP/server.py" "$CAPTURE" "$PORT_FILE" "$DOWN" "$LOSS" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT
for _ in $(seq 1 100); do [ -s "$PORT_FILE" ] && break; sleep .02; done
URL="http://127.0.0.1:$(cat "$PORT_FILE")"

wait_for() {
  local expr="$1"
  for _ in $(seq 1 300); do
    /usr/bin/python3 - "$CAPTURE" "$expr" <<'PY' && return 0
import json,sys
try: rows=[json.loads(line) for line in open(sys.argv[1]) if line.strip()]
except FileNotFoundError: rows=[]
raise SystemExit(0 if eval(sys.argv[2],{"rows":rows}) else 1)
PY
    sleep .05
  done
  echo "timed out waiting for hook requests: $expr" >&2
  return 1
}

HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
  bash scripts/hooks/hook-report.sh subagent-start subagent-start-hook \
  <<<'{"session_id":"abc-123","agent_id":"explore-1"}'
wait_for 'any(r["body"].get("agent_id")=="explore-1" for r in rows)'
/usr/bin/python3 - "$CAPTURE" <<'PY'
import json,sys
v=next(json.loads(line)["body"] for line in open(sys.argv[1]) if "explore-1" in line)
assert v["subagent"] == "start" and "state" not in v, v
assert v["source"] == "subagent-start-hook" and v["session_id"] == "abc-123", v
assert v["event_id"] == "abc-123:explore-1:start", v
print("ok   SubagentStart carries session, agent and stable event identity")
PY

# Two identity-less callbacks with identical payloads are two real agents, not
# duplicate delivery. Only retries of each queued row reuse its minted id.
for _ in 1 2; do
  HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
    bash scripts/hooks/hook-report.sh subagent-start subagent-start-hook <<<'{}'
done
wait_for 'len([r for r in rows if str(r["body"].get("event_id","")).startswith("anonymous:")]) >= 2'
/usr/bin/python3 - "$CAPTURE" <<'PY'
import json,sys
ids=[json.loads(line)["body"].get("event_id","") for line in open(sys.argv[1])]
ids=[i for i in ids if i.startswith("anonymous:")]
assert len(ids)>=2 and len(set(ids[-2:]))==2, ids
print("ok   two empty-payload starts mint distinct identities")
PY

# Response loss after the server read the event must retry the same identity.
touch "$LOSS"
HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
  bash scripts/hooks/hook-report.sh subagent-stop subagent-stop-hook \
  <<<'{"session_id":"abc-123","agent_id":"explore-loss"}'
wait_for 'len([r for r in rows if r["body"].get("agent_id")=="explore-loss"]) >= 2'
/usr/bin/python3 - "$CAPTURE" <<'PY'
import json,sys
rows=[json.loads(line)["body"] for line in open(sys.argv[1])]
rows=[r for r in rows if r.get("agent_id")=="explore-loss"]
assert rows[-2]["event_id"]==rows[-1]["event_id"], rows[-2:]
assert rows[-2]["delivery_attempt"]==1 and rows[-1]["delivery_attempt"]==2, rows[-2:]
print("ok   response-loss retry preserves event identity")
PY

# Outage recovery is FIFO: start reaches the rebuilt endpoint before stop.
touch "$DOWN"
for mode in start stop; do
  HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
    bash scripts/hooks/hook-report.sh "subagent-$mode" "subagent-$mode-hook" \
    <<EOF
{"session_id":"abc-123","agent_id":"ordered-agent"}
EOF
done
sleep .1; rm "$DOWN"
wait_for 'len([r for r in rows if not r["down"] and r["body"].get("agent_id")=="ordered-agent"]) >= 2'
/usr/bin/python3 - "$CAPTURE" <<'PY'
import json,sys
rows=[json.loads(line) for line in open(sys.argv[1])]
events=[r["body"]["subagent"] for r in rows if not r["down"] and r["body"].get("agent_id")=="ordered-agent"]
assert events[:2]==["start","stop"], events
print("ok   lifecycle start/stop replay in FIFO order after outage")
PY

# A permanent 4xx is a poison event, not an outage: dead-letter it with full
# identity and continue to the valid row behind it.
for agent in poison-agent after-poison; do
  HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
    bash scripts/hooks/hook-report.sh subagent-start subagent-start-hook \
    <<EOF
{"session_id":"abc-123","agent_id":"$agent"}
EOF
done
wait_for 'any(r["body"].get("agent_id")=="after-poison" for r in rows)'
grep -q 'lifecycle_queue=dead_letter.*verdict=non_retryable_http.*agent_id=poison-agent.*event=start' \
  "$TMP/home/.amux/logs/hook-report-failures.log"
echo "ok   poison 4xx dead-letters and does not head-of-line block the next event"

# A queue left by an expired drain is awakened by an ordinary prompt hook.
QF="$TMP/home/.amux/hook-report-queue/probe.json"
/usr/bin/python3 - "$QF" "$URL" <<'PY'
import json,os,sys
os.makedirs(os.path.dirname(sys.argv[1]),exist_ok=True)
body={"subagent":"start","source":"survivor","session_id":"abc-123",
      "agent_id":"surviving-queue","event_id":"surviving:event","event_ts":1.0}
json.dump([{"event_id":"surviving:event","body":body,"url":sys.argv[2],"attempts":90}],open(sys.argv[1],"w"))
PY
HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
  bash scripts/hooks/hook-report.sh active prompt-hook <<<'{}'
wait_for 'any(r["body"].get("agent_id")=="surviving-queue" for r in rows)'
wait_for 'any(r["body"].get("state")=="active" for r in rows)'
echo "ok   ordinary state hook wakes a surviving lifecycle queue"

# Compact is not a process reset; startup is. Pin both paths.
before=$(wc -l < "$CAPTURE")
HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
  bash scripts/hooks/hook-report.sh subagent-reset session-start-hook <<<'{"source":"compact","session_id":"abc-123"}'
sleep .1
test "$(wc -l < "$CAPTURE")" -eq "$before"
HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=probe \
  bash scripts/hooks/hook-report.sh subagent-reset session-start-hook <<<'{"source":"startup","session_id":"abc-123"}'
wait_for 'any(r["body"].get("subagent")=="reset" for r in rows)'
echo "ok   startup resets subagents while compact preserves them"

# The producer queue remains bounded under a prolonged outage and announces
# overflow in the hook failure log. Recover afterward so no drain is orphaned.
touch "$DOWN"
for _ in $(seq 1 12); do
  HOME="$TMP/home" AMUX_URL="$URL" AMUX_SESSION=bounded AMUX_HOOK_QUEUE_LIMIT=8 \
    bash scripts/hooks/hook-report.sh subagent-start subagent-start-hook <<<'{}'
done
/usr/bin/python3 - "$TMP/home/.amux/hook-report-queue/bounded.json" <<'PY'
import json,sys
rows=json.load(open(sys.argv[1])); assert len(rows)==8,len(rows)
assert len({r["event_id"] for r in rows})==8
print("ok   durable lifecycle queue obeys its bounded cap with distinct events")
PY
grep -q 'lifecycle_queue=overflow.*limit=8' "$TMP/home/.amux/logs/hook-report-failures.log"
rm "$DOWN"
for _ in $(seq 1 400); do
  pending=$(/usr/bin/python3 - "$TMP/home/.amux/hook-report-queue/bounded.json" <<'PY'
import json,sys
try: print(len(json.load(open(sys.argv[1]))))
except Exception: print(0)
PY
  )
  [ "$pending" -eq 0 ] && break
  sleep .05
done
test "$pending" -eq 0

echo "ok   all shipped status-hook durability regressions passed"
