"""The verify gate's dirty-checkout check, tested against the two specimens that
look identical to a content comparison and mean opposite things.

BACKE-3183 (backend). `git status` compares the working tree to the LOCAL HEAD. Lanes
that land work by pushing a commit-tree graft never move their local ref, so landed,
byte-identical files read ` M` forever and the verify gate refused checkouts that had
lost nothing. backend forced past it 4+ times in one night. The cost is not the
friction: routine forcing un-teaches the gate's one honest catch.

The proposed rule was "identical-to-origin is the only pass". That is necessary and
NOT sufficient, which these tests exist to pin:

  specimen 1  graft   HEAD stale, a.txt ` M`, blob-identical to upstream,
                      HEAD IS an ancestor of upstream   -> nothing local to lose, PASS
  specimen 2  revert  HEAD one unpushed commit AHEAD, tree put back to upstream content,
                      a.txt ` M`, ALSO blob-identical to upstream,
                      HEAD is NOT an ancestor            -> a destructive uncommitted
                                                            reversion, must stay DIRTY

Both are status-dirty AND identical-to-upstream. Content identity alone cannot tell
them apart, so a check built on it would bless specimen 2 and green-light discarding
committed work. The amux checkout carried 16 unpushed commits when this was written —
it is permanently in specimen 2's shape, so this is not a hypothetical.

test_specimen_2_* IS the self-test: it seeds the failure mode the naive rule would
have, and passes only if the guard refuses to clear it.
"""

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SERVER_PATH = REPO_ROOT / "amux-server.py"


@pytest.fixture(scope="module")
def srv():
    spec = importlib.util.spec_from_file_location("amux_server_dirt", SERVER_PATH)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_server_dirt"] = mod
    spec.loader.exec_module(mod)
    # No sibling session workdirs -> empty pathspec. Keeps the fixture repos from
    # depending on whatever the live fleet happens to be doing.
    mod._all_session_workdirs = lambda: {}
    return mod


def _git(wd, *args, **kw):
    r = subprocess.run(["git", "-C", str(wd)] + list(args),
                       capture_output=True, text=True, **kw)
    assert r.returncode == 0 or kw.get("ok_fail"), f"git {args} failed: {r.stderr}"
    return r.stdout.strip()


def _new_repo(tmp_path, name):
    """A bare 'origin' plus a clone with one base commit on a known branch."""
    origin = tmp_path / f"{name}-origin.git"
    wc = tmp_path / name
    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
    subprocess.run(["git", "clone", "-q", str(origin), str(wc)],
                   check=True, capture_output=True)
    _git(wc, "config", "user.email", "t@t")
    _git(wc, "config", "user.name", "t")
    _git(wc, "symbolic-ref", "HEAD", "refs/heads/main")
    (wc / "a.txt").write_text("upstream\n")
    (wc / "b.txt").write_text("keep\n")
    _git(wc, "add", "-A")
    _git(wc, "commit", "-qm", "base")
    _git(wc, "push", "-q", "-u", "origin", "main")
    return wc


def _make_graft(wc, path, content):
    """Land `content` on origin WITHOUT moving the local ref — the workflow that
    creates the false positive. Mirrors what the fleet does, rather than simulating
    the symptom by hand-editing a file."""
    blob = _git(wc, "hash-object", "-w", "--stdin", input=content)
    entries = []
    for f in sorted(p.name for p in Path(wc).iterdir() if p.suffix == ".txt"):
        h = blob if f == path else _git(wc, "rev-parse", f"HEAD:{f}")
        entries.append(f"100644 blob {h}\t{f}")
    tree = _git(wc, "mktree", input="\n".join(entries) + "\n")
    new = _git(wc, "commit-tree", tree, "-p", _git(wc, "rev-parse", "HEAD"),
               input="grafted\n")
    _git(wc, "push", "-q", "origin", f"{new}:main")
    _git(wc, "fetch", "-q", "origin")
    (Path(wc) / path).write_text(content)      # tree holds the landed content
    return new


# ─────────────────────────── specimen 1: the false positive ──────────────────

def test_specimen_1_graft_is_cleared(srv, tmp_path):
    wc = _new_repo(tmp_path, "graft")
    _make_graft(wc, "a.txt", "v2-landed\n")

    assert srv._session_dirty_files("s", str(wc)) == ["a.txt"], \
        "precondition: git status must call the grafted file dirty, else nothing is being tested"

    v = srv._upstream_dirt_verdicts("s", str(wc))
    assert v["head_is_upstream_ancestor"] is True
    assert v["already_upstream"] == ["a.txt"], v
    assert v["still_dirty"] == [], v
    assert "identical to" in v["verdicts"]["a.txt"]


def test_untouched_clean_repo_reports_nothing(srv, tmp_path):
    """Denominator guard: a clean repo must produce no verdicts either way, so a
    pass above cannot come from the function simply never finding files."""
    wc = _new_repo(tmp_path, "clean")
    v = srv._upstream_dirt_verdicts("s", str(wc))
    assert v["already_upstream"] == [] and v["still_dirty"] == []


# ────────────────── specimen 2: the case the naive rule would bless ──────────

def test_specimen_2_revert_is_NOT_cleared(srv, tmp_path):
    """THE SELF-TEST. Identical-to-upstream is true here too; only ancestry differs."""
    wc = _new_repo(tmp_path, "revert")
    (wc / "a.txt").write_text("important local work\n")
    _git(wc, "commit", "-qam", "local unpushed work")
    (wc / "a.txt").write_text("upstream\n")            # revert the tree, do not commit

    # the property that makes the naive rule wrong: content DOES match upstream
    assert _git(wc, "hash-object", "a.txt") == _git(wc, "rev-parse", "origin/main:a.txt")
    assert srv._session_dirty_files("s", str(wc)) == ["a.txt"]

    v = srv._upstream_dirt_verdicts("s", str(wc))
    assert v["head_is_upstream_ancestor"] is False, v
    assert v["already_upstream"] == [], (
        "a reverted-but-uncommitted file identical to upstream was cleared — this "
        "discards committed local work, which is the failure the ancestry guard exists "
        "to prevent")
    assert v["reason"] and "REVERTED" in v["reason"]


# ───────────────────────── things that must stay dirty ───────────────────────

def test_deletion_stays_dirty(srv, tmp_path):
    """backend's MG-1434 hazard: a removed file has no blob to match, and losing it
    is precisely what the gate should catch."""
    wc = _new_repo(tmp_path, "del")
    _make_graft(wc, "a.txt", "v2-landed\n")     # keep HEAD an ancestor so we DO adjudicate
    (Path(wc) / "b.txt").unlink()

    v = srv._upstream_dirt_verdicts("s", str(wc))
    assert v["head_is_upstream_ancestor"] is True, "guard must be open, else this proves nothing"
    assert "b.txt" in v["still_dirty"], v
    assert "DELETED" in v["verdicts"]["b.txt"]
    assert "a.txt" in v["already_upstream"], "the graft should still clear alongside a deletion"


def test_untracked_and_genuinely_modified_stay_dirty(srv, tmp_path):
    wc = _new_repo(tmp_path, "mixed")
    _make_graft(wc, "a.txt", "v2-landed\n")
    (Path(wc) / "new.txt").write_text("brand new\n")          # untracked
    (Path(wc) / "b.txt").write_text("genuinely edited\n")     # differs from upstream

    v = srv._upstream_dirt_verdicts("s", str(wc))
    assert set(v["still_dirty"]) == {"new.txt", "b.txt"}, v
    assert "untracked" in v["verdicts"]["new.txt"]
    assert "differs from" in v["verdicts"]["b.txt"]
    assert v["already_upstream"] == ["a.txt"]


def test_verdict_covers_every_dirty_path(srv, tmp_path):
    """The refusal quotes verdicts per path; a path with no verdict would render as
    an unexplained accusation."""
    wc = _new_repo(tmp_path, "cover")
    _make_graft(wc, "a.txt", "v2-landed\n")
    (Path(wc) / "new.txt").write_text("x\n")
    (Path(wc) / "b.txt").write_text("y\n")

    v = srv._upstream_dirt_verdicts("s", str(wc))
    covered = set(v["verdicts"])
    assert covered == set(v["already_upstream"]) | set(v["still_dirty"])
    assert covered == set(srv._session_dirty_files("s", str(wc))), \
        "verdicts must span exactly the files the gate would refuse on"
    print(f"checked {len(covered)} paths, all carry a verdict")
