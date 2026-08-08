"""The junk classifier must fire on cards that ARE artifacts, not cards ABOUT them (GCA-85).

general-canvas-apps, after being nudged three times to decompose-and-discard three valid
investigation cards: the title heuristic in _pickup_junk_reason anchored probe|temp|test
to the start of the title and left canary/tripwire/armed-watch floating, so any card
MENTIONING a canary was classified AS one. And the title rule returns before the
structure veto, so the protection built for exactly this case could never fire.

mixpeek-frustrations hit the same bug independently the same night: MF-523 — a real
merged fix in review, 2437-char desc, sha, tests — fired on 'tripwire' mid-title, while
its two near-identical siblings without the artifact word sailed through. The prescribed
exit was "discard MF-523", which would have destroyed the only record the fix shipped.
A nudge that says DISCARD is where the heuristic must be most conservative, because the
failure is irreversible and the compliant session pays it.

Titles below are the real specimens, copied as literals — no live-DB dependency (the
gate-contract suite was green for weeks on ambient state and red the moment CI ran on a
machine that had never started amux).
"""
import importlib.util
import os
import sys
import tempfile
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv():
    home = Path(tempfile.mkdtemp()) / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_junk_anchor", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_junk_anchor"] = mod
        spec.loader.exec_module(mod)
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


# Real specimen titles (truncation is faithful to what the classifier saw first).
MENTIONS = [
    "[fleet sweep] Do any OTHER scheduled detectors route their failures nowhere? demo-data-canary was",
    "[alerting gap] demo-data-canary failed 8 consecutive runs into total silence — a",
    "[api/collections] four SourceType values were accepted then failed with an impossible tripwire",
]
SUBJECTS = [
    "[probe] age-archive target",
    "Canary: demo-data freshness",
    "tripwire: fire when X regresses",
    "TEMP scratch for load test",
    "armed watch on the deploy gate",
]


@pytest.mark.parametrize("title", MENTIONS)
def test_a_card_ABOUT_an_artifact_is_not_classified_as_one(srv, title):
    """The regression. These are investigation cards whose SUBJECT is a detector; the
    artifact word appears mid-title. Pre-fix all three returned 'looks like a test
    artifact or armed tripwire' and the nudge prescribed decompose-and-discard."""
    why = srv._pickup_junk_reason(title, "THE DEFECT: x\nCONSEQUENCE: y\nFIX: z")
    assert why == "", (
        "a card MENTIONING an artifact word mid-title is classified as an artifact "
        "again — the unanchored alternatives are back (GCA-85): %r" % why)


@pytest.mark.parametrize("title", SUBJECTS)
def test_a_card_that_IS_an_artifact_still_fires(srv, title):
    """The counter-case that keeps the anchor honest: when the artifact word IS the
    subject (start of title), the card is dormant and must still be refused —
    dormancy deliberately beats structure, it just has to actually BE dormancy."""
    why = srv._pickup_junk_reason(title, "short desc")
    assert why == "looks like a test artifact or armed tripwire", (
        "a genuinely dormant subject no longer fires: %r -> %r" % (title, why))


def test_mid_title_mention_loses_to_structure_even_without_heads(srv):
    """MF-523's exact shape: artifact word mid-title, structured desc. The structure
    veto must now be reachable for it (pre-fix the title rule returned first)."""
    why = srv._pickup_junk_reason(
        "[api/collections] values failed with an impossible tripwire",
        "ROOT CAUSE: the enum drifted.\nFix merged as 7535398ac3.")
    assert why == "", why
