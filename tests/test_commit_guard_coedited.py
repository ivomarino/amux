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

    def run(chk, files=("amux-server.py",)):
        sent.clear()
        m._commit_guard_nudged.pop("lane", None)
        m._commit_guard_daily.clear()
        m._session_dirty_files = lambda n, w, _f=list(files): list(_f)
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
    assert "--check" in t, (
        "no per-hunk staging recipe, so the only actionable reading is still "
        "`git add <file>`, which takes the whole file")
    assert "--unidiff-zero" not in t or "Do NOT use" in t, (
        "the warning recommends --unidiff-zero as the recipe again. That flag DISABLES "
        "context checking, and recommending it here produced four unbuildable commits on "
        "2026-08-08 (3bf1470, 01b5d02, 9188a9f, 10916ac) — a trimmed zero-context patch "
        "lands at a stale offset exactly when the removed and kept hunks share a region, "
        "which is the only case this warning fires in")
    assert "ast.parse" in t, (
        "the recipe no longer tells the reader to verify the STAGED blob. The pre-commit "
        "hook parsed the WORKING TREE, so partial staging could commit unparseable code "
        "with the check reporting success — do not rely on the hook alone")


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


def test_partly_foreign_gets_the_DO_NOT_COMMIT_warning(guard):
    """amux's `NOT YOURS` block, promoted from a source-shape assertion to a behavioural one.

    Distinct from the co-edited case and not covered by it: `foreign` means the recipient did
    NOT touch that path at all, so the instruction is "do not commit this", not "stage your
    hunks". With a second file that IS theirs, the nudge must still fire — suppressing would
    strand the recipient's own real work.
    """
    m_files = ["peer_only.py", "mine.py"]
    t = guard({"ok": True,
               "foreign": [{"path": "peer_only.py", "owner": "amux", "age_secs": 90}],
               "shared": [], "cotenants": ["amux"]},
              files=m_files)
    assert t, "a partly-foreign tree must still nudge — the recipient has real work in mine.py"
    assert "NOT YOURS" in t, "no do-not-commit warning for a file the recipient never touched"
    note = t.split("NOT YOURS")[1]
    assert "peer_only.py" in note and "amux" in note, "the warning names neither the file nor the owner"
    assert "in-flight" in t or "Do not commit" in t


def test_both_warnings_can_appear_together(guard):
    """One file purely a peer's, another co-edited. Both notes must survive — they are
    different instructions (do not commit vs stage only your hunks), and the second one
    appends to the first, so a naive assignment instead of += would silently drop one."""
    t = guard({"ok": True,
               "foreign": [{"path": "peer_only.py", "owner": "amux", "age_secs": 90}],
               "shared": [{"path": "shared.py", "owner": "amux", "age_secs": 120}],
               "cotenants": ["amux"]},
              files=["peer_only.py", "shared.py", "mine.py"])
    assert "NOT YOURS" in t and "CO-EDITED" in t, (
        "one of the two warnings was overwritten rather than appended")


def test_shared_entries_for_files_already_dropped_are_ignored(guard):
    """`shared` is intersected with the files that SURVIVED the foreign filter. Without
    that, a path dropped as foreign could still contribute a warning naming a file the
    message no longer lists — a notice pointing at something the reader cannot see."""
    t = guard({"ok": True,
               "foreign": [{"path": "other.py", "owner": "peer", "age_secs": 30}],
               "shared": [{"path": "other.py", "owner": "peer", "age_secs": 30}],
               "cotenants": ["peer"]})
    assert "other.py" not in t.split("CO-EDITED")[-1] if "CO-EDITED" in t else True


def test_unclaimed_staged_paths_get_their_own_verdict():
    """AMUX-2443 / AF-19 residual / AC-297 — three sightings of one gap: a
    staged path with NO edit record from ANY session fell through both the
    shared and foreign branches and was never mentioned, which is how a peer's
    pre-window staged file (762e06e's sweep) shipped silently. It cannot be
    BLOCKED (no owner to defer to; possibly the committer's own pre-window
    work) — the verdict is a loud list, present with consistent shape on every
    return path. Drives the REAL classifier (the `guard` fixture stubs it)."""
    spec = importlib.util.spec_from_file_location("amux_cg_unclaimed", SRC)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_cg_unclaimed"] = mod
    spec.loader.exec_module(mod)
    mod._all_session_workdirs = lambda: {"me": "/tmp/repo", "peer": "/tmp/repo"}
    mod._repo_root = lambda p: "/tmp/repo"
    mod._session_recent_edit_paths = lambda s, w, firsthand_only=False: {}
    d = mod._staged_guard_check("me", "/tmp/repo", ["ghost.py"])
    assert "unclaimed" in d, "shape: the key must exist on the populated path"
    assert [f["path"] for f in d["unclaimed"]] == ["ghost.py"], d
    assert d["foreign"] == [] and d["shared"] == []
    # Shape consistency on the cotenant-less early return too.
    mod._all_session_workdirs = lambda: {"me": "/tmp/repo"}
    d2 = mod._staged_guard_check("me", "/tmp/repo", ["ghost.py"])
    assert "unclaimed" in d2 and d2["unclaimed"] == []
