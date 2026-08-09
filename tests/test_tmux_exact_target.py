"""tmux -t targets must be exact ('=name'), never bare — bare is a PREFIX match.

AMUX-2544/2555 ("figure out why social-media keeps stopping"): an archived
session named `social` coexisted with the live `social-media`. Every few
minutes _enforce_archived_stopped ran `tmux kill-session -t amux-social`;
tmux resolves a bare -t by exact-then-unique-prefix, found no amux-social,
prefix-matched amux-social-media, and killed Ethan's freshly-started worker
61 seconds after it started. The log said "reaped 1 orphan ... social" —
naming the archived session, not the victim — so the kill was invisible from
both ends: no stop event for social-media, and a plausible-looking cleanup
line for social.

The same resolution rule applies to EVERY -t: send-keys aimed at a bare
prefix name would type into the longer-named session (the cross-delivery
shape of AMUX-1730). So the fix is at the seam — tmux_target() returns
'=name' — and this test pins every subprocess call site to the exact form.
"""

import re
import shutil
import subprocess
import uuid
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


def test_no_bare_tmux_name_reaches_a_dash_t_flag():
    """Every subprocess -t argument must be tmux_target(...) (exact by
    construction) or an explicit '=' + name. new-session -s is creation, not
    matching, and legitimately stays raw."""
    src = SERVER_PATH.read_text()
    bad = []
    for m in re.finditer(r'"-t",\s*(tmux_name\([^)]*\)|tmux_sess\b|f"[^"]*")', src):
        line_no = src[:m.start()].count("\n") + 1
        bad.append("line %d: %s" % (line_no, m.group(0)))
    assert not bad, (
        "bare (prefix-matching) tmux -t target(s) — an archived short name "
        "kills/types-into the live longer name (AMUX-2544): %s" % bad)


def test_tmux_target_is_exact_with_pane_colon():
    """tmux_target must return '=name:' — not just '=name'.

    The '=' forces exact session match (no prefix). The trailing ':' is
    required for pane-targeting commands (capture-pane, send-keys, pipe-pane,
    respawn-pane, display-message). Without it, tmux returns 'can't find pane'
    and exits 1. Session-level commands (has-session, kill-session) tolerate
    both forms, which is why the original test passed while every pane
    operation across 62 sessions was silently failing.
    """
    src = SERVER_PATH.read_text()
    m = re.search(r"def tmux_target\(.*?\n(.*?)\n\ndef ", src, re.S)
    assert m, "tmux_target function not found"
    body = m.group(0)
    assert 'return "=" + tmux_name(session) + ":"' in body, (
        "tmux_target must return '=name:' — the colon resolves the pane "
        "within the exact-matched session. Without it, capture-pane/send-keys/"
        "pipe-pane all fail with 'can\'t find pane' (the 2026-08-08 outage)")


@pytest.mark.skipif(shutil.which("tmux") is None, reason="tmux not installed")
def test_the_prefix_hazard_is_real_and_the_exact_form_averts_it():
    """Behavioral proof against a real tmux server, so the rule is pinned to
    tmux's actual resolution semantics rather than to our reading of the man
    page. Creates only throwaway uniquely-named sessions and removes them."""
    tag = "amuxtest-" + uuid.uuid4().hex[:8]
    longer = tag + "-longer"
    try:
        subprocess.run(["tmux", "new-session", "-d", "-s", longer, "-x", "20", "-y", "5"],
                       capture_output=True, timeout=10, check=True)
        # Bare short name: tmux prefix-matches the longer session — this IS the bug.
        bare = subprocess.run(["tmux", "has-session", "-t", tag],
                              capture_output=True, timeout=5)
        assert bare.returncode == 0, (
            "tmux no longer prefix-matches a bare -t; if this genuinely "
            "changed upstream, the '=' hardening is redundant — delete this "
            "test only with a tmux changelog citation")
        # Exact form: no match, no cross-session damage.
        exact = subprocess.run(["tmux", "has-session", "-t", "=" + tag],
                               capture_output=True, timeout=5)
        assert exact.returncode != 0, "'=' exact form matched a non-existent session"
        # And the kill that motivated all of this: exact kill of the SHORT name
        # must NOT take down the longer session.
        subprocess.run(["tmux", "kill-session", "-t", "=" + tag],
                       capture_output=True, timeout=5)
        still = subprocess.run(["tmux", "has-session", "-t", "=" + longer],
                               capture_output=True, timeout=5)
        assert still.returncode == 0, (
            "exact kill-session of the short name killed the longer session")
    finally:
        subprocess.run(["tmux", "kill-session", "-t", "=" + longer],
                       capture_output=True, timeout=5)


@pytest.mark.skipif(shutil.which("tmux") is None, reason="tmux not installed")
def test_pane_commands_need_colon_after_exact_match():
    """The original test only verified session commands (has-session, kill-session).
    Pane commands (capture-pane, send-keys, pipe-pane) require '=name:' — without
    the trailing colon they fail with 'can't find pane' even though the session
    exists. This gap caused the 2026-08-08 outage: every capture and send-keys
    across all 62 sessions was silently failing."""
    tag = "amuxtest-" + uuid.uuid4().hex[:8]
    try:
        subprocess.run(["tmux", "new-session", "-d", "-s", tag, "-x", "20", "-y", "5"],
                       capture_output=True, timeout=10, check=True)
        # '=name' alone: session exists, but pane commands fail
        cap_bare = subprocess.run(
            ["tmux", "capture-pane", "-t", "=" + tag, "-p"],
            capture_output=True, text=True, timeout=5)
        assert cap_bare.returncode != 0, (
            "'=name' without colon should fail for capture-pane — if tmux "
            "changed this behavior upstream, the colon is redundant but harmless")
        # '=name:' — the correct form for pane commands
        cap_colon = subprocess.run(
            ["tmux", "capture-pane", "-t", "=" + tag + ":", "-p"],
            capture_output=True, text=True, timeout=5)
        assert cap_colon.returncode == 0, (
            "'=name:' should succeed for capture-pane")
        # send-keys with colon must also work
        sk = subprocess.run(
            ["tmux", "send-keys", "-t", "=" + tag + ":", ""],
            capture_output=True, timeout=5)
        assert sk.returncode == 0, "'=name:' should succeed for send-keys"
    finally:
        subprocess.run(["tmux", "kill-session", "-t", "=" + tag],
                       capture_output=True, timeout=5)


def test_raw_pipe_pane_sites_use_colon():
    """pipe-pane call sites that bypass tmux_target() and build the target
    directly as '=' + tmux_name(name) must also include the trailing colon."""
    src = SERVER_PATH.read_text()
    bad = []
    for m in re.finditer(r'"=" \+ tmux_name\([^)]*\)(?!\s*\+\s*":")', src):
        line_no = src[:m.start()].count("\n") + 1
        line = src.splitlines()[line_no - 1].strip()
        if "pipe-pane" in src[max(0, m.start()-200):m.start()]:
            bad.append("line %d: %s" % (line_no, line))
    assert not bad, (
        "pipe-pane site(s) using '=' + tmux_name without trailing colon — "
        "pipe-pane is a pane command and needs '=name:': %s" % bad)
