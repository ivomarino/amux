"""The archived-but-unfinished surfacing must reach BUSY lanes, not only idle ones.

Ethan, 2026-08-07, on AMUX-2499: "figure out why this hasn't been done".

`archived=1 AND status NOT IN (verified, discarded)` is a card no autonomy loop can
see: every one of them filters `COALESCE(archived,0)=0`, correctly, because that is
what stops cleared work leaking back into auto-pickup. 507ccb6 added a per-lane
notification so the invisibility at least gets announced.

It was placed in `_advance_open_card`'s `if not row:` tail — the branch reached only
when a lane has NOTHING advanceable. Limbo cards accumulate fastest on the lanes with
the most activity, so the check ran precisely where the population was smallest.

Measured on the live fleet before the fix:

  25 notifications ever sent, all within one 9-minute window, all to small lanes.
  54 lanes were holding limbo cards at the time.
  Of the top twelve holders, the only two that could reach the branch were the two
  with zero advanceable candidates.
  Ten lanes holding 1235 cards — 86% of the population — were structurally
  unreachable by the mechanism built to surface them.

This is the second defect of exactly this shape in this one function: AC-194 hoisted
the stale-ask re-nag out of the same tail after 48 cycles and 0 fires. Hence a test
rather than another fix: "put it where the loop has nothing else to do" keeps looking
like politeness and keeps being a filter on the population you most need to reach.

The tests seed a lane with BOTH advanceable work and limbo cards — the combination
that was unreachable, and the one a convenient fixture would omit, since the easy
fixture is a lane with nothing else going on.
"""

import importlib.util
import os
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture
def srv(tmp_path):
    home = tmp_path / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_limbo", None)
        spec = importlib.util.spec_from_file_location("amux_server_limbo", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_limbo"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        # The lane must exist as an env file or _advance_open_card bails at parse.
        (home / "sessions" / "busylane.env").write_text("CC_DIR=/tmp\n")
        # Capture instead of delivering. Returns (ok, err) like the real one.
        mod._sent = []
        mod.send_text = lambda name, text, **kw: (mod._sent.append((name, text)), (True, ""))[1]
        # The ordinary advance nudge is gated on `is_running` (a real tmux pane);
        # a synthetic lane is not running, so without this the plain-nudge
        # counter-case below cannot send and the test would fail for a reason
        # unrelated to what it checks. Found by running these against the
        # PRE-CHANGE code: that one test failed there too, which is what
        # distinguishes a fixture gap from a regression. The other three failed
        # only pre-change — that is the part that proves the fix.
        mod.is_running = lambda _n: True
        mod._advance_last.clear()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def _issue(srv, iid, status, archived=0, session="busylane", owner="agent"):
    now = int(time.time())
    srv.get_db().execute(
        "INSERT INTO issues (id,title,status,session,type,created,updated,owner_type,archived) "
        "VALUES (?,?,?,?,'code',?,?,?,?)",
        (iid, "t " + iid, status, session, now, now, owner, archived))
    srv.get_db().commit()


def test_a_busy_lane_still_hears_about_its_limbo_cards(srv):
    """THE REGRESSION. A lane with advanceable work AND limbo cards must be told.

    Pre-fix this lane took the `row` branch on the doing card and returned before the
    limbo check existed in its path — so the more work a lane had, the less likely it
    was to ever learn that some of it was unreachable.
    """
    _issue(srv, "L-1", "doing")                    # advanceable: takes the row branch
    _issue(srv, "L-2", "todo", archived=1)         # limbo
    _issue(srv, "L-3", "review", archived=1)       # limbo
    srv._advance_open_card("busylane")
    msgs = [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t]
    assert msgs, (
        "a lane holding advanceable work never hears about its archived-but-unfinished "
        "cards — the check is back in the idle-only tail (AMUX-2499)")
    assert "2 of your cards" in msgs[0], msgs[0][:200]


def test_an_idle_lane_still_hears_too(srv):
    """The case that already worked must keep working. A relocation that fixes busy
    lanes by breaking idle ones has moved the blind spot, not removed it."""
    _issue(srv, "L-4", "todo", archived=1)
    srv._advance_open_card("busylane")
    assert [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t]


def test_limbo_is_checked_BEFORE_the_advanceable_selection(srv):
    """Ordering, asserted through behaviour rather than by reading the source.

    With both present the limbo message must be what goes out on this pass — if the
    advance nudge wins, the limbo notice waits for a pass where nothing is
    advanceable, which is the original defect with extra steps.
    """
    _issue(srv, "L-5", "doing")
    _issue(srv, "L-6", "todo", archived=1)
    srv._advance_open_card("busylane")
    assert srv._sent, "nothing was sent at all"
    assert "ARCHIVED but not finished" in srv._sent[0][1], (
        "the advanceable card was nudged first; limbo waits for an idle pass: %s"
        % srv._sent[0][1][:160])


def test_a_lane_with_no_limbo_cards_is_not_delayed(srv):
    """The counter-case that stops the test above passing vacuously: a lane with only
    normal work must still get its ordinary advance nudge. A limbo check that swallowed
    every pass would 'fix' this by silencing the loop."""
    _issue(srv, "L-7", "doing")
    srv._advance_open_card("busylane")
    assert srv._sent, "a lane with plain advanceable work got no nudge at all"
    assert "ARCHIVED but not finished" not in srv._sent[0][1]


def test_dedupe_is_keyed_on_the_DAY_not_the_count(srv):
    """The key was `limbo:<lane>:<count>`, wrong in both directions simultaneously.

    Count moves by one -> new key -> re-notify, so a lane is nagged for making
    progress (live evidence: amux-homepage told at 16 and again at 6, two minutes
    apart). Count returns to a seen value -> that key is burnt -> silent forever with
    the condition unresolved (cold-outbound fired at 14 and at 10).

    Here: notify, then CHANGE the count, then run again. One message, because the day
    has not changed.
    """
    _issue(srv, "L-8", "todo", archived=1)
    srv._advance_open_card("busylane")
    assert len(srv._sent) == 1, srv._sent
    _issue(srv, "L-9", "todo", archived=1)          # count 1 -> 2
    srv._advance_last.clear()                        # ignore the cooldown, isolate dedupe
    srv._advance_open_card("busylane")
    limbo = [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t]
    assert len(limbo) == 1, (
        "a changed count produced a second notification — the key is count-derived "
        "again, so a lane gets nagged for working the pile down: %d sent" % len(limbo))


def test_count_keyed_dedupe_would_have_re_fired(srv):
    """Pins WHY the key changed, so the old scheme is not restored as a tidy-up.
    Same scenario, evaluating the OLD key shape: it produces two distinct keys."""
    old = lambda lane, n: "limbo:%s:%d" % (lane, n)
    assert old("busylane", 1) != old("busylane", 2), (
        "the count-keyed scheme no longer distinguishes — if this fails, delete this "
        "test rather than the day-keyed fix")
    new = lambda lane, t: "limbo:%s:%d" % (lane, int(t // 86400))
    t = time.time()
    assert new("busylane", t) == new("busylane", t + 60), (
        "the day-keyed scheme re-fires within a day, which is the noise mode it "
        "exists to remove")


def test_the_message_reports_the_TOTAL_not_the_sampled_page(srv):
    """mvs-infra, 2026-08-07: the notice showed 8 ids then '+12 more' — read by anyone
    as 20 — against an actual 182, because N came from a LIMIT-20 row set. The count
    must be its own query. Seeded past the cap so a sample-derived total is wrong."""
    for i in range(25):
        _issue(srv, "M-%d" % i, "todo", archived=1)
    srv._advance_open_card("busylane")
    msg = [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t][0]
    assert "25 of your cards" in msg, (
        "the total is derived from the capped sample, understating the population: %s"
        % msg[:160])
    assert "+17 more" in msg, msg[:250]


def test_human_owned_limbo_cards_are_surfaced_not_swept(srv):
    """298 of these are owner_type=human. The notice must still name them — the lane
    is being asked to look, not to act — while nothing anywhere force-moves them
    (ethos rule 8). Guards against a future 'fix' that resolves the count by
    auto-closing, which is the leak the archived filter exists to prevent."""
    _issue(srv, "H-1", "todo", archived=1, owner="human")
    srv._advance_open_card("busylane")
    assert [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t]
    st = srv.get_db().execute("SELECT status, archived FROM issues WHERE id='H-1'").fetchone()
    assert st[0] == "todo" and st[1] == 1, (
        "a human-owned card was mutated by the surfacing pass: %s" % (tuple(st),))


def test_an_expired_rate_limit_is_not_reported(srv):
    """AMUX-2566 (Ethan: "some are just workers I haven't touched in a long time").

    The stored limit flag cleared only on ACTIVITY EVIDENCE in the pane, which an
    untouched worker never produces — so the flag outlived its own reset time
    indefinitely, and card badges rendering on truthiness kept the lane dressed as
    rate-limited for days. A rate limit is a claim about TIME; the payload now
    gates it on the clock at the one choke point every render site reads."""
    import time as _t
    (Path(srv.CC_SESSIONS) / "rlprobe.env").write_text("CC_DIR=/tmp\n")
    srv._session_auto_actions["rlprobe"] = {
        "rate_limit_reset_at": _t.time() - 3600, "rate_limit_weekly": True}
    s = [x for x in srv.list_sessions() if x["name"] == "rlprobe"][0]
    assert s["rate_limited_until"] == 0, (
        "an hour-expired limit is still reported — untouched workers stay badged "
        "(AMUX-2566): %r" % s["rate_limited_until"])
    assert s["rate_limit_weekly"] is False

    # Counter-case: a GENUINE future limit must still be reported, or the fix
    # "cures" the false badge by killing the true one.
    fut = _t.time() + 3600
    srv._session_auto_actions["rlprobe"]["rate_limit_reset_at"] = fut
    s = [x for x in srv.list_sessions() if x["name"] == "rlprobe"][0]
    assert s["rate_limited_until"] == fut and s["rate_limit_weekly"] is True


def _typed_issue(srv, iid, status, itype, archived=1):
    now = int(time.time())
    srv.get_db().execute(
        "INSERT INTO issues (id,title,status,session,type,created,updated,owner_type,archived) "
        "VALUES (?,?,?,?,?,?,?,'agent',?)",
        (iid, "t " + iid, status, "busylane", itype, now, now, archived))
    srv.get_db().commit()


def test_done_is_terminal_for_noncode_types_but_not_for_code(srv):
    """AC-310: a doc report or declined escalation ENDS at done — its done gate
    is the terminal claim ("Outcome recorded") and verified is optional
    re-confirmation. The sweep re-nagged three resolved AC cards daily forever
    because no honest action could satisfy its predicate: verified would assert
    what never happened, discarded would erase what did. Meanwhile code's done
    is merged-but-unconfirmed (Ethan's done!=verified rule) and MUST still
    surface. Untyped cards default code — strictest — and must surface too."""
    srv._sent.clear()
    srv._advance_last.clear()
    srv.get_db().execute("DELETE FROM issues")
    srv.get_db().execute("DELETE FROM session_events WHERE idem LIKE 'limbo:%'")
    srv.get_db().commit()
    _typed_issue(srv, "T-DOC", "done", "doc")            # terminal — must NOT surface
    _typed_issue(srv, "T-ESC", "done", "escalation")     # terminal — must NOT surface
    _typed_issue(srv, "T-CODE", "done", "code")          # unconfirmed — MUST surface
    _typed_issue(srv, "T-NOTYPE", "done", "")            # defaults code — MUST surface
    _typed_issue(srv, "T-RTODO", "todo", "research")     # unfinished at any type — MUST surface
    srv._advance_open_card("busylane")
    msgs = [t for (_n, t) in srv._sent if "ARCHIVED but not finished" in t]
    assert msgs, "the limbo notice stopped firing entirely — over-filtered (the sign flips, ethos rule 1)"
    m = msgs[0]
    assert "T-DOC" not in m and "T-ESC" not in m, (
        "a non-code done card still counts as limbo — the daily forever-nag is back: %s" % m[:200])
    assert "T-CODE" in m and "T-NOTYPE" in m and "T-RTODO" in m, (
        "the fix over-filtered: a genuinely unfinished card vanished from the notice: %s" % m[:200])
    assert "3 of your cards" in m, "count disagrees with the sample's predicate: %s" % m[:120]
