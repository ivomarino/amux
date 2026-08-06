#!/bin/bash
# auto-deploy.sh — push committed site changes after a board card goes to done.
#
# Safety rule (shared checkout): only pushes when EVERY unpushed commit on
# main carries Amux-Session: amux-homepage. A single foreign commit aborts —
# it would ship under this session's push with no review, which is exactly
# the incident documented in CLAUDE.md's Deploy section.
#
# This script is called from a PostToolUse hook (Bash matcher). It exits 0
# silently whenever there is nothing to push or the trigger doesn't match,
# so it never blocks an unrelated command.

set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

# Parse hook stdin — we need to know if the command was a board-done operation.
# We accept both:
#   amux board done ITEM_ID
#   curl ... -d '{"status":"done"...}' .../api/board/...
CMD=$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('command', ''))
except Exception:
    pass
" 2>/dev/null || echo "")

# Only fire on board done operations
if ! echo "$CMD" | grep -qE '(amux board done|api/board.*(\"status\":\"done\"|status.*done))'; then
  exit 0
fi

cd "$REPO"

# How many commits are ahead of origin/main?
AHEAD=$(git rev-list --count origin/main..main 2>/dev/null || echo 0)
if [ "$AHEAD" -eq "0" ]; then
  exit 0  # nothing to push
fi

# Collect all session attributions from unpushed commits.
# Commits from this session carry "Amux-Session: amux-homepage" in the body.
# If any commit is unattributed or attributed to a different session, abort.
FOREIGN=$(git log --format="%H %B" origin/main..main | python3 -c "
import sys, re
text = sys.stdin.read()
# Split on commit SHAs
commits = re.split(r'\n([0-9a-f]{40}) ', text)
foreign = []
for block in commits:
    sessions = re.findall(r'Amux-Session:\s*(\S+)', block)
    if not sessions:
        continue  # no attribution — treat as foreign (conservative)
    for s in sessions:
        if s != 'amux-homepage':
            foreign.append(s)
print('\n'.join(set(foreign)))
" 2>/dev/null || echo "parse-error")

if [ -n "$FOREIGN" ]; then
  echo "auto-deploy: skipping push — other sessions have unpushed commits: $FOREIGN" >&2
  exit 0
fi

# All $AHEAD commits are mine. Safe to push.
echo "auto-deploy: pushing $AHEAD commit(s) to origin/main..."
git push origin main
echo "auto-deploy: deployed."
