"""The verification sweep must not burn its cooldown before doing the work (AF-13).

It used to write and COMMIT one global `verify_sweep_last` before the per-lane loop.
list_sessions() and _status_applies() sit OUTSIDE the per-lane try, so an exception in
either aborted the pass with the cooldown already spent — every lane after the failure
point silently waited another 20h, in list_sessions() order, behind a single slog line.

That was survivable while the advance loop also nudged `done` cards. AMUX-2565 removed
that tier on Ethan's directive, so this sweep is now the ONLY clock on done->verified and
a half-finished pass has no backstop.

The fix is per-lane stamping, written AFTER each lane's work. These tests pin the two
properties that make it a fix rather than a rearrangement:

  1. the CUTOVER does not re-sweep the fleet (a missing per-lane key inherits the last
     global sweep) — "fixing a filter is a migration event", and the first run after a
     gate change is where that bites
  2. a lane that was never reached is NOT stamped, so it retries
"""
import importlib.util
import json
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
        sys.modules.pop("amux_server_vsweep", None)
        spec = importlib.util.spec_from_file_location("amux_server_vsweep", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_vsweep"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def _pref(mod, key):
    row = mod.get_db().execute("SELECT value FROM prefs WHERE key=?", (key,)).fetchone()
    return json.loads(row[0]) if row else None


def test_the_cutover_does_not_resweep_a_fleet_that_was_just_swept(srv):
    """THE MIGRATION CASE. NOT a fix-detector — verified against a pre-fix specimen and
    it passes there too, because the old global gate produced the same silence. It is a
    REGRESSION guard on this change: without the inherit-the-global seed, the first pass
    after deploy re-sweeps the whole fleet. Before this change there were no per-lane keys at all. If a
    missing key meant "never swept", the first pass after deploy would treat every live
    lane as due and message the whole fleet at once — a backlog discharge dressed as a
    day's work. A missing key must inherit the last GLOBAL sweep instead."""
    now = int(time.time())
    db = srv.get_db()
    # The fleet was swept an hour ago, globally, under the OLD scheme.
    db.execute("INSERT INTO prefs (key, value) VALUES ('verify_sweep_last', ?)",
               (json.dumps(now - 3600),))
    db.commit()
    sent = []
    srv._steer_enqueue = lambda name, text, **kw: sent.append(name)
    srv.list_sessions = lambda: [{"name": "lane-a", "running": True, "archived": 0},
                                 {"name": "lane-b", "running": True, "archived": 0}]
    srv._status_applies = lambda status, name: (True, "")
    srv._verification_sweep()
    assert sent == [], (
        "the cutover re-swept lanes that were globally swept an hour ago — a missing "
        "per-lane key is being read as 'never swept' (AF-13)")


def test_a_lane_past_its_own_cooldown_is_swept_and_then_stamped(srv):
    """Counter-case: the migration guard must not wedge the sweep shut forever. A lane
    whose cooldown HAS elapsed still gets its message, and is stamped afterwards."""
    now = int(time.time())
    db = srv.get_db()
    db.execute("INSERT INTO prefs (key, value) VALUES ('verify_sweep_last', ?)",
               (json.dumps(now - 30 * 3600),))
    db.execute(
        "INSERT INTO issues (id,title,desc,status,session,type,created,updated,notified,"
        "                    owner_type,archived) "
        "VALUES ('VS-1','a done card','','done','lane-a','code',?,?,1,'agent',0)",
        (now - 100, now - 100))
    db.commit()
    sent = []
    srv._steer_enqueue = lambda name, text, **kw: sent.append(name)
    srv.list_sessions = lambda: [{"name": "lane-a", "running": True, "archived": 0}]
    srv._status_applies = lambda status, name: (True, "")
    srv._verification_sweep()
    assert sent == ["lane-a"], "a lane 30h past its cooldown was not swept: %r" % (sent,)
    assert _pref(srv, "verify_sweep_last:lane-a") is not None, (
        "the lane was messaged but never stamped — it will be re-messaged every pass")


def test_a_lane_the_pass_never_reached_is_left_unstamped(srv):
    """THE FIX ITSELF. The pass dies partway; lanes it never got to must NOT carry a
    fresh cooldown, or they lose a full 20h cycle to work that never happened. Before
    the fix a single global stamp was already committed, so EVERY lane looked swept."""
    now = int(time.time())
    db = srv.get_db()
    db.execute("INSERT INTO prefs (key, value) VALUES ('verify_sweep_last', ?)",
               (json.dumps(now - 30 * 3600),))
    db.commit()
    srv.list_sessions = lambda: [{"name": "lane-a", "running": True, "archived": 0},
                                 {"name": "lane-b", "running": True, "archived": 0}]

    def _boom(status, name):
        if name == "lane-b":
            raise RuntimeError("simulated mid-loop failure")
        return (True, "")

    srv._status_applies = _boom
    srv._steer_enqueue = lambda name, text, **kw: None
    srv._verification_sweep()          # must swallow, not raise

    # BOTH halves, because the negative alone is vacuous: against the PRE-FIX code no
    # per-lane key exists for anyone, so "lane-b is unstamped" is trivially true and the
    # test passes on the very defect it exists to catch. Checked against a real pre-fix
    # specimen (git show HEAD:amux-server.py) — the lane-a assertion is what goes red
    # there, and it is what makes the pair discriminating.
    assert _pref(srv, "verify_sweep_last:lane-a") is not None, (
        "the lane processed BEFORE the failure was not stamped — either the stamp is "
        "still global (pre-fix) or it is not written per lane at all (AF-13)")
    assert _pref(srv, "verify_sweep_last:lane-b") is None, (
        "a lane the pass never processed was stamped anyway — it now waits out a 20h "
        "cooldown for work that did not happen (AF-13)")


def test_the_audit_counters_survive_an_early_exception(srv):
    """Also not a fix-detector (pre-fix there were no counters to be unbound). It guards
    THIS change against someone moving the bindings back inside the try.

    _due/_completed are read by the except handler. Bound inside the try, an early
    failure would raise NameError *inside the handler written to record the failure* —
    the exact defect AMUX-2349 fixed one variable over in this same function."""
    srv.list_sessions = lambda: (_ for _ in ()).throw(RuntimeError("early boom"))
    db = srv.get_db()
    db.execute("INSERT INTO prefs (key, value) VALUES ('verify_sweep_last', ?)",
               (json.dumps(int(time.time()) - 30 * 3600),))
    db.commit()
    srv._verification_sweep()          # must not raise NameError out of the handler
