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
required={"UserPromptSubmit","PostToolUse","Stop","SubagentStart","SubagentStop"}
assert required <= set(hooks)
rows=[]
for event, groups in hooks.items():
    for group in groups:
        for hook in group.get("hooks", []):
            rows.append((event, group.get("matcher"), hook.get("command", "")))
reports=[r for r in rows if "hook-report.sh" in r[2]]
assert len(reports) == 5, reports
assert len([r for r in reports if r[0] == "PostToolUse" and r[1] == ".*"]) == 1
assert any(r[2] == "echo unrelated" for r in rows)
assert any(r[2] == "bash check-format.sh" for r in rows)
assert not any("/api/sessions/" in r[2] and "hook-report.sh" not in r[2] for r in rows)
print("ok   installer is idempotent and preserves unrelated hooks/settings")
PY

# Drive the shipped shell hook with a fake curl that captures the exact JSON
# submitted to /report. No live amux instance is touched.
mkdir -p "$TMP/bin" "$TMP/home/.amux/logs"
cat > "$TMP/bin/curl" <<'SH'
#!/usr/bin/env bash
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "-d" ]]; then
    shift
    printf '%s' "$1" > "$HOOK_CAPTURE"
  fi
  shift || true
done
printf '200'
SH
chmod +x "$TMP/bin/curl"

HOOK_CAPTURE="$TMP/subagent.json" HOME="$TMP/home" PATH="$TMP/bin:$PATH" \
  AMUX_SESSION=probe bash scripts/hooks/hook-report.sh \
  subagent-start subagent-start-hook <<<'{"session_id":"abc-123"}'
/usr/bin/python3 - "$TMP/subagent.json" <<'PY'
import json, sys
v=json.load(open(sys.argv[1]))
assert v["subagent"] == "start", v
assert "state" not in v, v
assert v["source"] == "subagent-start-hook", v
assert v["session_id"] == "abc-123", v
print("ok   SubagentStart emits an attributed subagent=start report")
PY

HOOK_CAPTURE="$TMP/prompt.json" HOME="$TMP/home" PATH="$TMP/bin:$PATH" \
  AMUX_SESSION=probe bash scripts/hooks/hook-report.sh \
  active prompt-hook <<<'{}'
/usr/bin/python3 - "$TMP/prompt.json" <<'PY'
import json, sys
v=json.load(open(sys.argv[1]))
assert v["state"] == "active", v
assert "subagent" not in v, v
assert v["source"] == "prompt-hook", v
print("ok   UserPromptSubmit still emits state=active")
PY
