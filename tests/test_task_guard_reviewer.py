"""The idle task-guard must not nag a lane for work it did as a REVIEWER (AF-15).

The guard's two suppressions both ask `WHERE session=?` — "does this lane own a card
that is doing / recently closed?". That is the right question for an author and
structurally blind to a reviewer, because review->done and done->verified land on the
AUTHOR's card: a reviewer never owns the card they close.

Measured incident, 2026-08-08: the nudge fired three times in one afternoon at
amux-frustrations while five cards (AMUX-2542/2553/2562/2565/2566) carried its
reviewer sign-off, every one owned by `amux`. There was no truthful way to comply —
a card titled "reviewed someone else's card" is not a unit of work that can be
honestly done or not done, which is the placeholder-card outcome the sibling
suppression's own docstring exists to prevent.

These tests pin the SUPPRESSION and, just as importantly, its counter-cases: a
suppression that never lets the nudge through would "fix" this by disabling the guard.
"""
import importlib.util
import os
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = str(Path(__file__).parent.parent / "amux-server.py")


@pytest.fixture
def srv(tmp_path):
    home = tmp_path / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_taskguard", None)
        spec = importlib.util.spec_from_file_location("amux_server_taskguard", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_taskguard"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        (home / "sessions" / "reviewerlane.env").write_text("CC_DIR=/tmp\n")
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def _card(mod, cid, status, session, reviewer=None, age_s=0):
    db = mod.get_db()
    ts = int(time.time()) - age_s
    db.execute(
        "INSERT INTO issues (id,title,desc,status,session,type,created,updated,notified,"
        "                    owner_type,archived,reviewer) "
        "VALUES (?,?,'',?,?,'code',?,?,1,'agent',0,?)",
        (cid, f"title for {cid}", status, session, ts, ts, reviewer))
    db.commit()
    return cid


def test_a_reviewers_signoff_on_someone_elses_card_suppresses_the_nudge(srv):
    """The incident specimen, rebuilt from the real cards: the reviewer owns
    NOTHING, and the card it signed off belongs to `amux`."""
    _card(srv, "AMUX-2566", "done", session="amux", reviewer="reviewerlane")
    assert srv._session_recently_reviewed_issue("reviewerlane") is True, (
        "a lane that just signed off a peer's card reads as having done nothing — "
        "the guard is still asking `WHERE session=?` (AF-15)")


def test_done_to_verified_counts_too(srv):
    """An independent verify is the same shape of work on someone else's card, and
    it is what the VERIFY nudges ask for by name."""
    _card(srv, "AMUX-2381", "verified", session="amux", reviewer="reviewerlane")
    assert srv._session_recently_reviewed_issue("reviewerlane") is True


def test_the_suppression_does_NOT_fire_for_a_lane_that_reviewed_nothing(srv):
    """Counter-case. Without this, "always return True" would pass the test above
    and silently disable the guard for every lane."""
    _card(srv, "AMUX-9001", "done", session="amux", reviewer="someone-else")
    _card(srv, "AMUX-9002", "done", session="reviewerlane", reviewer=None)
    assert srv._session_recently_reviewed_issue("reviewerlane") is False


def test_a_stale_signoff_does_not_suppress_forever(srv):
    """Counter-case on the window: yesterday's review is not evidence about today."""
    _card(srv, "AMUX-8000", "done", session="amux", reviewer="reviewerlane",
          age_s=srv._TASK_GUARD_CLOSED_WINDOW + 600)
    assert srv._session_recently_reviewed_issue("reviewerlane") is False


def test_an_unfinished_review_does_not_count(srv):
    """A card still sitting in `review` has not been signed off — the reviewer is
    the one who still owes the work, so the nudge is correct to fire."""
    _card(srv, "AMUX-7000", "review", session="amux", reviewer="reviewerlane")
    assert srv._session_recently_reviewed_issue("reviewerlane") is False


def test_the_guard_itself_stays_silent_for_a_reviewer(srv):
    """End-to-end through _task_guard, not just the helper — the helper being right
    is worth nothing if it is never consulted (the mechanism-exists trap)."""
    srv._session_auto_actions.clear()
    _card(srv, "AMUX-2565", "done", session="amux", reviewer="reviewerlane")
    sent = []
    srv.send_text = lambda name, text, **kw: (sent.append((name, text)), (True, ""))[1]
    srv.is_running = lambda _n: True
    os.environ["AMUX_TASK_GUARD"] = "1"
    srv._task_guard_nudged.pop("reviewerlane", None)
    srv._task_guard_last.pop("reviewerlane", None)
    try:
        fired = srv._task_guard("reviewerlane")
    finally:
        os.environ.pop("AMUX_TASK_GUARD", None)
    assert fired is False and not sent, (
        "the guard nudged a lane whose only recent work was signing off a peer's "
        "card: %r" % (sent,))
