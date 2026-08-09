"""The co-edit sweep notice must re-check its claim at delivery (AF-21).

Its load-bearing sentence is "you edited it at HH:MM and have not committed it since
HH:MM". That fact expires. Three notices on 2026-08-08 asserted an 18:33 last-commit that
commit 44bd9fe had already superseded at 19:36 — all three TRUE when emitted (their commits
landed 19:06 / 19:14 / 19:15) and false by the time they were read.

It matters more than an ordinary stale nudge because of what it ASKS: your work may have
been swept into someone else's commit, so a false positive sends the reader auditing a
commit that contains none of it. And a stale one is indistinguishable from the real thing —
762e06e genuinely did carry another session's staged work under the identical sentence.

_steer_guard_stale already re-checks dep:/verify:/decompose:/ctx:/unverified:/review:/rot:/
sched:/watch: at delivery. The sweep notice enqueues with `swept:<sha>:<session>` and had no
handler, so it was the one perishable-git-state nudge left out of that sweep (c32cf8a,
7504abf).

The counter-cases matter as much as the suppression: a guard that always returns stale
would silence the alarm entirely, which is strictly worse than the noise it fixes.
"""
import importlib.util
import os
import sys
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
        sys.modules.pop("amux_server_swept", None)
        spec = importlib.util.spec_from_file_location("amux_server_swept", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_swept"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_the_swept_guard_is_actually_wired(srv):
    """The whole defect was that `swept:` fell through to the unknown-guard default.
    A handler that exists but is never reached is the same bug with more code."""
    assert srv._steer_guard_stale("nolane", "swept:") == (False, ""), \
        "an empty sha must not be treated as stale"
    # An unknown guard is the control: if THIS returns something different from a
    # swept: guard on a lane with no work dir, the prefix is being routed.
    assert srv._steer_guard_stale("nolane", "totally-unknown-guard") == (False, "")


def test_a_lane_with_no_work_dir_still_gets_the_warning(srv):
    """FAIL LOUD. If the check cannot run, the notice must still speak — suppressing
    a sweep alarm because the checker broke is the false negative that costs real work."""
    srv._session_work_dir = lambda n: ""
    stale, why = srv._steer_guard_stale("lane-a", "swept:deadbee:lane-a")
    assert stale is False, "suppressed the alarm when it could not verify: %r" % (why,)


def test_an_exception_in_the_check_does_not_swallow_the_notice(srv, monkeypatch):
    """Same direction, different cause: git blowing up must not silence the warning.

    monkeypatch, NOT `srv.subprocess.run = ...`. `srv.subprocess` is the SHARED stdlib
    module object, so assigning through it mutates subprocess.run process-wide and never
    restores — my first version did exactly that and broke 8 unrelated tests in
    test_tmux_exact_target and test_upstream_dirt, which shell out. The failure surfaced
    as "git exploded" raised from a tmux call, i.e. blamed on innocent code."""
    srv._session_work_dir = lambda n: "/nonexistent/repo/path"

    def _boom(*a, **k):
        raise RuntimeError("git exploded")

    monkeypatch.setattr(srv.subprocess, "run", _boom)
    stale, why = srv._steer_guard_stale("lane-a", "swept:deadbee:lane-a")
    assert stale is False, "an error in the re-check silenced the notice: %r" % (why,)


def test_the_guard_uses_the_emitters_predicate_not_a_looser_one(srv):
    """THE CORRECTION. My first implementation asked "has this lane committed this path
    recently" — a different question, which answers YES for a genuine sweep too and would
    have suppressed 762e06e, the one notice that day that was real.

    The emitter's predicate is `your last edit <= your last commit of that path`. This
    pins that the guard reads the lane's EDIT time, so a lane that edited a path AFTER its
    last commit of that path is still warned."""
    src = open(SERVER_PATH).read()
    i = src.find('if guard.startswith("swept:"):')
    assert i > 0, "the swept: handler is gone — re-anchor this test"
    # Window sized from the handler's real extent, not guessed: my first cut used
    # 3000 chars and missed the comparison, which is the same positional-probe
    # mistake this repo keeps paying for. Bound it at the NEXT guard instead.
    _end = src.find('if guard.startswith("verify:")', i)
    assert _end > i, "cannot bound the swept: handler — re-anchor"
    block = src[i:_end]
    assert "_session_recent_edit_paths" in block, (
        "the swept: guard no longer consults the lane's EDIT times, so it cannot be "
        "applying the emitter's `edit <= commit` predicate (AF-21)")
    assert "_edit <= _cts" in block, (
        "the guard stopped comparing edit-time against commit-time — a re-check that does "
        "not share the emitter's predicate answers a different question")
