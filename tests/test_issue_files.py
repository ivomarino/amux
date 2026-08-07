"""Issue context files are POINTERS, not a blob store (AMUX-2508).

Ethan named attachments as the gap when he shared the reference board: their issue
detail carries task-definition.md, research-notes.md, gates-hierarchy.json and so on,
each with size and provenance.

The design constraint is the whole test file. amux already has FILESYSTEM and BOARD as
primitives, so an attachment is the JOIN between them — not a ninth thing. Storing
bytes in the board would double every artifact into a database the fleet syncs and make
the board the owner of content the filesystem already owns, which is a second spelling
of an existing primitive.

Two properties follow and both are pinned here:
  detaching must never delete the file  — the board does not own it
  a pointer that cannot be opened is worse than none — so attach REFUSES a bad path,
  and list REPORTS a row whose file has since vanished rather than hiding it
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
        sys.modules.pop("amux_server_files", None)
        spec = importlib.util.spec_from_file_location("amux_server_files", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_files"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        mod._TEST_HOME = home        # AMUX_HOME is restored below; keep it reachable
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_table_stores_a_path_not_bytes(srv):
    """The design constraint, asserted against the schema rather than trusted: if a
    blob column ever appears here, the board has started owning filesystem content."""
    cols = [r[1] for r in srv.get_db().execute("PRAGMA table_info(issue_files)")]
    assert "path" in cols
    for forbidden in ("blob", "content", "data", "bytes"):
        assert forbidden not in cols, (
            f"issue_files grew a `{forbidden}` column — attachments are pointers; "
            "storing content makes the board a second filesystem")


def test_absolute_path_resolves(srv, tmp_path):
    f = tmp_path / "notes.md"
    f.write_text("x")
    got = srv._resolve_issue_file(str(f), {"session": "whoever"})
    assert got == os.path.realpath(str(f))


def test_relative_path_resolves_against_the_OWNING_lane(srv, tmp_path):
    """Not the server's cwd. The server runs from wherever launchd started it, and a
    card's context belongs to the lane that attached it — resolving against cwd would
    silently point at a different repo."""
    wd = tmp_path / "repo"
    wd.mkdir()
    (wd / "research.md").write_text("y")
    (Path(srv._TEST_HOME) / "sessions" / "ln.env").write_text(f"CC_DIR={wd}\n")
    got = srv._resolve_issue_file("research.md", {"session": "ln"})
    assert got == os.path.realpath(str(wd / "research.md"))


def test_relative_path_with_no_lane_does_not_fall_back_to_cwd(srv):
    """The counter-case. Falling back to the server's cwd would resolve to a real file
    in the amux checkout and look like it worked — a pointer confidently aimed at the
    wrong repo is worse than one that fails."""
    assert srv._resolve_issue_file("frustrations.md", {"session": ""}) == ""


def test_a_vanished_file_is_reported_missing_not_hidden(srv, tmp_path):
    """A pointer table goes stale when files move. Dropping those rows from the listing
    would make the panel look clean while context the card CLAIMS to carry is gone —
    the same 'unreachable is what a reader experiences as deleted' failure as the
    memory archive, one primitive over."""
    f = tmp_path / "gone.md"
    f.write_text("z")
    resolved = srv._resolve_issue_file(str(f), {"session": "x"})
    assert os.path.isfile(resolved)
    f.unlink()
    assert srv._resolve_issue_file(str(f), {"session": "x"}) == os.path.realpath(str(f))
    assert not os.path.isfile(resolved), "the listing must be able to report exists=False"
