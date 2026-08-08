"""Secrets must never ride the herdr command line (AMUX-2571).

The first herdr e2e (AMUX-2554) found workspace create receiving
--env KEY=VALUE pairs as argv — live OPENAI/GOOGLE/LOB keys readable by any
local process via ps for the duration of the call. The tmux path never does
this: env flows through the shell. The fix writes a 0600 file of exports,
sources it into the pane shell in the same pane-run that pins the cwd, and
the file's own last line deletes it — which doubles as the boot handshake
(the file vanishing is the only proof a booting pane gives that it executed
anything; its agent_status reads "unknown" throughout).
"""

import re
from pathlib import Path

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


def test_no_env_pair_reaches_herdr_argv():
    """The construction that leaked: `ws_args += ["--env", pair]` (or any
    --env forwarding of env_pairs). Grep the shipped source — if --env
    returns as an argv vehicle for pairs, the ps leak is back."""
    src = SERVER_PATH.read_text()
    bad = []
    for m in re.finditer(r'"--env"', src):
        line_no = src[:m.start()].count("\n") + 1
        line = src.split("\n")[line_no - 1].strip()
        bad.append("line %d: %s" % (line_no, line[:80]))
    assert not bad, (
        "--env argv construction present — secret values ride the herdr "
        "command line again (AMUX-2571): %s" % bad)


def test_the_env_file_is_0600_selfdeleting_and_quoted():
    """The three properties that make the file path safe, pinned in source:
    restrictive open flags, a self-delete last line, and shlex quoting of
    VALUES (a key containing '$(...)' or quotes must reach the shell inert)."""
    src = SERVER_PATH.read_text()
    seg = src[src.find("herdr-env-"):]
    seg = seg[:4000]
    assert "os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600" in seg, (
        "env file no longer created with 0600 — a group/world-readable file "
        "of provider keys is the ps leak with extra steps")
    assert 'rm -f -- ' in seg, "self-delete handshake line missing"
    assert "shlex.quote(_ev)" in seg, (
        "values no longer shell-quoted — a value with $() or quotes executes "
        "in the pane shell")


def test_the_handshake_waits_and_fails_loudly():
    """A dropped keystroke into a booting shell is silent; an agent started
    without creds fails later in a way that does not name this step. The
    consumption wait plus the loud give-up line are the difference between a
    diagnosable miss and another afternoon of (no-json)-class archaeology."""
    src = SERVER_PATH.read_text()
    assert "env file never consumed by the pane shell" in src
