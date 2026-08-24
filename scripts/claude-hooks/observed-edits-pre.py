#!/usr/bin/env python3
# observed-edits PRE half (AF-123). Marks t0 before every Bash command so the
# POST half can report files whose mtime moved during it. Observed, not
# parsed: 75% of AF-27 staged-guard blocks hit lanes with firsthand=0, because
# bypass-permissions lanes are told to edit through Bash and no pathlike regex
# can see a write spelled inside a heredoc or aimed at an extensionless file
# (the specimen: a python heredoc rewriting `amux`).
#
# TRACKED SOURCE: scripts/claude-hooks/observed-edits-pre.py. Installed to
# ~/.amux/hooks/ and wired in ~/.claude/settings.json (PreToolUse, matcher
# "Bash"). Fail-open always: a hook that can block Bash fleet-wide must never
# have a failure mode of its own.
import os
import subprocess
import sys


def _derive_session_from_tmux():
    """Fallback identity for MR-43: $AMUX_SESSION can be empty inside a lane
    that IS running in its amux-launched pane (spawn always injects it —
    session_verbs.rs — so this is loss in-process, not absence at launch).
    Scoped to amux- prefixed panes, so a human's own tmux session (or no tmux
    at all) still resolves to "" and takes the existing no-op path.
    """
    try:
        name = subprocess.run(["tmux", "display-message", "-p", "#S"],
                              capture_output=True, text=True, timeout=3).stdout.strip()
    except Exception:
        return ""
    return name[len("amux-"):] if name.startswith("amux-") else ""


try:
    session = (os.environ.get("AMUX_SESSION") or "").strip()
    if not session:
        session = _derive_session_from_tmux()
    if session:
        state = os.path.join(
            os.environ.get("AMUX_HOME") or os.path.expanduser("~/.amux"),
            "hooks", "state")
        os.makedirs(state, exist_ok=True)
        # touch: the marker's MTIME is t0
        with open(os.path.join(state, f"observed-{session}.t0"), "w") as fh:
            fh.write("")
except Exception:
    pass
sys.exit(0)
