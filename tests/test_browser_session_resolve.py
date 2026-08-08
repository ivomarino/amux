"""AC-293: /api/browser/* must not put every caller in the lane literally named "amux".

All eight browser endpoints did `body.get("session", "amux")` and no browser endpoint
read X-Amux-Session anywhere. The fallback is not a neutral sentinel — "amux" is a real,
live lane — so every caller that omitted `session` silently drove that lane's driver,
page and cookie jar. Two lanes doing it are ONE session as far as this API is concerned,
and whoever navigates last wins.

Reproduced before fixing: POST /api/browser/start with X-Amux-Session: amux-cloud
returned backend="driver", then POST /api/browser/action with session="amux-cloud"
returned backend="cli", because the driver had registered under "amux". The eval fell
through to the browser-use CLI — a different browser process (AMUX-2272) — and returned
a page nobody had loaded, as a success.

These tests exercise the SHIPPED resolver rather than a reimplementation, so a change
that reverts the precedence fails here even if the source-shape checks pass.
"""
import importlib.util
import os
import re
import sys
import tempfile
from pathlib import Path

SRC_PATH = Path(__file__).parent.parent / "amux-server.py"
SRC = SRC_PATH.read_text()


def _mod():
    """Load amux-server.py under a throwaway AMUX_HOME (same recipe as
    test_browser_driver_lock.py, so two test files cannot fight over real state)."""
    home = Path(tempfile.mkdtemp()) / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_busess", SRC_PATH)
        m = importlib.util.module_from_spec(spec)
        sys.modules["amux_busess"] = m
        spec.loader.exec_module(m)
        return m
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


class H(dict):
    """Stands in for self.headers — .get() is the whole interface used."""


def test_header_is_used_when_no_explicit_session():
    """THE REGRESSION. A lane that stamps its identity the way CLAUDE.md mandates must
    get its OWN browser, not the lane named "amux"."""
    assert _mod()._bu_session_resolve(None, H({"X-Amux-Session": "amux-cloud"})) == "amux-cloud"


def test_explicit_session_BEATS_the_header():
    """Driving another lane's browser stays possible — it just has to be said out loud.
    If the header won, a reviewer could never watch a peer's page."""
    m = _mod()
    assert m._bu_session_resolve("other-lane", H({"X-Amux-Session": "amux-cloud"})) == "other-lane"


def test_bare_amux_fallback_is_UNCHANGED():
    """The dashboard sends no header and passes its own session explicitly, so it must be
    byte-for-byte unaffected. Changing this fallback would be a separate behaviour change
    smuggled into a bug fix."""
    assert _mod()._bu_session_resolve(None, H({})) == "amux"


def test_blank_and_whitespace_are_not_a_session():
    """An empty string must not win over the header — `{"session": ""}` is a caller that
    omitted it, not a caller asking for a lane named ""."""
    m = _mod()
    for empty in ("", "   ", "\t"):
        assert m._bu_session_resolve(empty, H({"X-Amux-Session": "amux-cloud"})) == "amux-cloud", (
            "%r was treated as an explicit session" % empty)
    assert m._bu_session_resolve(None, H({"X-Amux-Session": "  "})) == "amux", (
        "a whitespace header was treated as a real lane name")


def test_non_string_explicit_does_not_crash_or_win():
    """A JSON body can carry anything. `{"session": 5}` must fall through, not raise —
    a 500 here takes out every browser call for that lane."""
    m = _mod()
    for junk in (5, [], {}, True):
        assert m._bu_session_resolve(junk, H({"X-Amux-Session": "lane"})) == "lane"


def test_no_browser_endpoint_still_hardcodes_the_amux_DEFAULT():
    """The whole point is that this was EIGHT sites, not one. A single missed endpoint
    keeps the collision alive on whichever verb it serves, and the symptom (a page you
    never loaded, returned as success) does not name the endpoint that caused it."""
    block = SRC[SRC.index('if method == "POST" and path == "/api/browser/start"') - 4000:]
    block = block[:block.index('def _bu_agent_run') if 'def _bu_agent_run' in block else 40000]
    bad = [l.strip() for l in block.splitlines()
           if re.search(r'session\s*=\s*(?:body|qs)\.get\(\s*"session"\s*,\s*\[?"amux"', l)]
    assert not bad, ("browser endpoints still defaulting session to the literal live lane "
                     '"amux": %r' % bad)


def test_amux_agent_namespace_is_left_alone():
    """_bu_agent_run's "amux-agent" default is NOT this bug and must not be swept into the
    fix: it is a deliberately separate namespace for the Computer Use loop, and no lane is
    named that. Recorded so a later cleanup does not "finish the job" and break it."""
    assert 'def _bu_agent_run(task: str, session: str = "amux-agent"' in SRC
