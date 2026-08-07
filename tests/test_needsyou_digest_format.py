"""The owner's SMS digest must say what to DO, and must not cut mid-word (AMUX-2526).

Ethan, 2026-08-07, from a screenshot of his phone: "text alerts appear cutoff also they
need to be summarized with action and worker id source."

Two defects in one line of code — `title[:70]`:

  CUT MID-WORD, unmarked. His phone showed "...go through the same durable queu". A
  hard slice with no ellipsis means a truncated line is indistinguishable from a line
  that simply ends, so the reader cannot tell they are missing something. Same rule as
  the board list having to declare its cap.

  WRONG FIELD. A card TITLE says what the work IS. An alert to the owner exists to say
  what HE has to do. `amux board needsyou` records the actual question as a NEEDS-YOU:
  line in the desc, written by whoever blocked on him — that is the action, and it was
  being ignored in favour of the title.

Worker id was already present and correct; these tests pin it so a rewrite cannot drop
the one part that worked.
"""

import importlib.util
import os
import re
import sys
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True, exist_ok=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_server_digest", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_digest"] = mod
        spec.loader.exec_module(mod)
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_never_cuts_mid_word(srv):
    """The reported symptom, rebuilt from his screenshot: the real line ended
    'durable queu'."""
    real = ("Offline audio recording + file upload go through the same durable queue, "
            "with the dictation outbox unified into the upload path")
    out = srv._ellipsize(real, 70)
    assert out.endswith("…"), f"truncation is unmarked: {out!r}"
    body = out.rstrip("…")
    assert not real.startswith(body + "e"), "cut mid-word — 'queue' became 'queu'"
    # every emitted word must be a whole word from the source
    for w in body.split():
        assert w in real.split(), f"{w!r} is not a whole word from the source"


def test_short_text_is_untouched(srv):
    """The counter-case: a rule that always appends an ellipsis is not truncation,
    it is decoration, and it would make every alert look clipped."""
    assert srv._ellipsize("short one", 150) == "short one"


def test_newlines_are_collapsed_not_sent_raw(srv):
    """A multi-line ask inside a bulleted SMS breaks the bullet structure, which is
    what makes a batched digest readable at all."""
    assert "\n" not in srv._ellipsize("line one\n\nline two", 150)


def test_the_digest_prefers_the_RECORDED_ASK_over_the_title(srv):
    """The 'summarized with action' half. Asserted against the source because the
    digest sends SMS and hits the DB; what matters is which field it reaches for."""
    src = SERVER_PATH.read_text()
    i = src.find("def _needsyou_digest")
    block = src[i:i + 6000]
    assert "NEEDS[- ]YOU:" in block or "NEEDS-YOU:" in block, (
        "the digest no longer looks for the recorded ask — it is back to sending the "
        "card title, which says what the work is and not what the owner must do")
    assert "_ellipsize(" in block, "the digest no longer word-safe truncates"


def test_worker_id_source_is_still_in_the_line(srv):
    """The one part that already worked. Pinned so a rewrite cannot silently drop it —
    an ask with no worker attached is one the owner cannot route."""
    src = SERVER_PATH.read_text()
    i = src.find("def _needsyou_digest")
    block = src[i:i + 6000]
    assert "r['session'] or 'unowned'" in block, (
        "the digest line no longer names the owning worker")
