"""AC-300: the idle nudge must not tell a session to commit a peer's in-flight work.

ROOT CAUSE, and it is shared with AC-297. _staged_guard_check classifies a path that BOTH
sessions edited as `shared`, not `foreign` — deliberately, since the committer has a claim
to their own hunks. But amux is a SINGLE-FILE project, so "both edited amux-server.py" is
satisfied essentially always (that function's own comment says so). The commit-guard's
foreign-dirt filter consumed only `foreign`, so on the one path with all the collisions it
was inert, and the nudge told me to commit amux's mid-iteration hook matchers.

One function, two consumers, OPPOSITE failures: the same `ap in mine` exemption made the
staged guard silent on a commit that swept ~85 lines of a peer's work (AC-297) and made this
nudge loud about work that was not mine. That is why "two independent instruments" was the
wrong diagnosis — the disagreement is between the consumers' predicates, not in the
classifier.

These drive the SHIPPED _commit_guard with the classifier stubbed to each of its three real
output shapes, because the bug was never in the classifier — it was in what the consumer
read. A source-shape test would have passed throughout.
"""
import importlib.util
import os
import sys
import tempfile
from pathlib import Path

import pytest

SRC = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture
def guard():
    """The real _commit_guard with everything around it stubbed.

    _push_alert and _emit_event are stubbed to no-ops on purpose: this test must not fire a
    real alert into the fleet. is_running is stubbed True because the enqueue is gated on it
    and the first version of this test silently sent nothing for that reason — a false
    negative that looked like the feature being absent.
    """
    home = Path(tempfile.mkdtemp()) / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_cg", SRC)
        m = importlib.util.module_from_spec(spec)
        sys.modules["amux_cg"] = m
        spec.loader.exec_module(m)
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev

    m._session_work_dir = lambda n: "/repo"
    m._session_dirty_files = lambda n, w: ["amux-server.py"]
    m._checkout_busy_cotenant = lambda n, w: None
    m._commit_guard_session_enabled = lambda n: True
    m.is_running = lambda n: True
    m._push_alert = lambda *a, **k: None
    m._emit_event = lambda *a, **k: None
    sent = {}
    m._steer_enqueue = lambda name, text, **kw: sent.__setitem__(name, text)

    def run(chk):
        sent.clear()
        m._commit_guard_nudged.pop("lane", None)
        m._commit_guard_daily.clear()
        m._staged_guard_check = lambda s, w, f: chk
        m._commit_guard("lane")
        return sent.get("lane", "")

    return run


def test_coedited_file_gets_a_contested_warning(guard):
    """THE INCIDENT. foreign=[] with shared populated is the shape a single-file repo
    produces essentially always, and it is what told me to commit a peer's work."""
    t = guard({"ok": True, "foreign": [],
               "shared": [{"path": "amux-server.py", "owner": "amux", "age_secs": 120}],
               "cotenants": ["amux"]})
    assert t, "no nudge at all — suppressing on `shared` would silence this guard "\
              "permanently on a single-file repo, which is the opposite over-correction"
    assert "CO-EDITED" in t, (
        "the nudge still reads as 'this is your work, commit it' for a file a peer is "
        "mid-edit in — that is AC-300 unchanged")
    note = t.split("CO-EDITED")[1]
    assert "amux" in note[:120], "the warning does not name WHO else is in the file"
    assert "leave it" in t, (
        "no instruction for the case where NONE of it is the recipient's — which was the "
        "actual situation, and the nudge's own WIP-checkpoint advice would have committed "
        "a peer's known-broken intermediate")
    assert "--unidiff-zero" in t, (
        "no per-hunk staging recipe, so the only actionable reading is still "
        "`git add <file>`, which takes the whole file")


def test_all_foreign_still_does_not_nudge(guard):
    """Pre-existing behaviour must survive: if every dirty path is purely a peer's, stay
    quiet. Regression guard on the filter I edited."""
    assert guard({"ok": True,
                  "foreign": [{"path": "amux-server.py", "owner": "amux", "age_secs": 60}],
                  "shared": [], "cotenants": ["amux"]}) == ""


def test_solo_dirty_tree_gets_NO_false_warning(guard):
    """The note must not become boilerplate. A session alone in the checkout has no
    contested hunks, and a warning that always fires stops being read."""
    t = guard({"ok": True, "foreign": [], "shared": [], "cotenants": []})
    assert t, "a genuinely solo dirty tree should still be nudged"
    assert "CO-EDITED" not in t


def test_shared_entries_for_files_already_dropped_are_ignored(guard):
    """`shared` is intersected with the files that SURVIVED the foreign filter. Without
    that, a path dropped as foreign could still contribute a warning naming a file the
    message no longer lists — a notice pointing at something the reader cannot see."""
    t = guard({"ok": True,
               "foreign": [{"path": "other.py", "owner": "peer", "age_secs": 30}],
               "shared": [{"path": "other.py", "owner": "peer", "age_secs": 30}],
               "cotenants": ["peer"]})
    assert "other.py" not in t.split("CO-EDITED")[-1] if "CO-EDITED" in t else True
