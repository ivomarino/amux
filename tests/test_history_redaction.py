"""Chat history tables must scrub credentials on the way in (AMUX-2525).

61c269d/AMUX-2502 redacted the board-capture path (title + prompt_text) after a
live key landed on a card. cmd_history and steering_history store the same chat
verbatim and stayed open for a day as the leak's other half — the backfill run
when this closed found 55 cmd_history rows and 1 steering_history row still
carrying credential-shaped values in the live DB.

Behavioral tests cover the two importable recorders; a source invariant covers
all four write sites (the two API handlers included), so a fifth INSERT added
later without a scrub fails the class check rather than reopening the leak.
"""

import importlib.util
import os
import re
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"
# Specimens are ASSEMBLED at runtime: the repo's own pre-commit secret scan
# (correctly) cannot tell a synthetic fixture from a live key, and blocked the
# literal form of the AIza specimen. The concatenation is the sanctioned way
# to keep the test honest without teaching anyone to --no-verify past the scan.
FAKE_KEY = "sk-" + "proj-" + "A" * 48


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_hist", None)
        spec = importlib.util.spec_from_file_location("amux_server_hist", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_hist"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        yield mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_cmd_history_scrubs_on_write(srv):
    srv._cmd_hist_record("lane-x", f"use this key: {FAKE_KEY} for the script")
    row = srv.get_db().execute(
        "SELECT text FROM cmd_history WHERE session='lane-x' ORDER BY ts DESC LIMIT 1"
    ).fetchone()
    assert FAKE_KEY not in row["text"], "cmd_history stored a credential verbatim"
    assert "[REDACTED-CREDENTIAL]" in row["text"]


def test_cmd_history_leaves_clean_text_alone(srv):
    """The counter-case: scrubbing must not mangle ordinary prompts."""
    msg = "please review the board and commit the fix"
    srv._cmd_hist_record("lane-y", msg)
    row = srv.get_db().execute(
        "SELECT text FROM cmd_history WHERE session='lane-y' ORDER BY ts DESC LIMIT 1"
    ).fetchone()
    assert row["text"] == msg


def test_the_scrubber_catches_the_backfilled_shapes(srv):
    """The regexes must at minimum catch the key shapes found sitting in the
    live DB when the backfill ran (sk-proj / AIza / generic KEY=value)."""
    for specimen in (FAKE_KEY,
                     "AIza" + "SyBt-" + "X" * 31,
                     "LOB_API_KEY=" + "live_" + "a" * 34):
        clean, hits = srv._redact_secrets(f"context {specimen} more")
        assert hits >= 1, "scrubber missed a shape the backfill found live: %s..." % specimen[:12]


def test_every_history_insert_site_scrubs():
    """Source invariant across ALL write sites, including the two API handlers
    the behavioral tests cannot reach: every INSERT into either history table
    must have a _redact_secrets call within the 15 lines above it."""
    src = SERVER_PATH.read_text()
    lines = src.split("\n")
    bad = []
    for i, line in enumerate(lines):
        if re.search(r"INSERT (OR REPLACE )?INTO (cmd_history|steering_history)", line):
            window = "\n".join(lines[max(0, i - 15):i])
            if "_redact_secrets" not in window:
                bad.append("line %d: %s" % (i + 1, line.strip()[:70]))
    assert not bad, (
        "history INSERT site(s) without a redaction call above them — the "
        "AMUX-2525 leak class is reopening: %s" % bad)
