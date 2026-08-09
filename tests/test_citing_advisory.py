"""The review->done citing advisory: informs, never decides (AC-302/AMUX-2578).

AC-302 killed every ENFORCING discriminator against its own specimens — a
docs commit is indistinguishable from an implementation commit, and an
uncited fix produces no signal — so the shipped shape surfaces the fact and
decides nothing: "commits citing <id> carry Amux-Session: X (n), Y (m)". The
would-be self-acker who sees their own name can decline (amux-cloud did,
twice, by hand); a legitimate reviewer proceeds.
"""

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv():
    spec = importlib.util.spec_from_file_location("amux_server_cite", SERVER_PATH)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_server_cite"] = mod
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture()
def repo(tmp_path):
    wc = tmp_path / "r"
    wc.mkdir()
    def g(*a, **kw):
        return subprocess.run(["git", "-C", str(wc)] + list(a),
                              capture_output=True, text=True, check=True, **kw)
    g("init", "-q")
    g("config", "user.email", "t@t"); g("config", "user.name", "t")
    (wc / "f").write_text("1"); g("add", "-A")
    g("commit", "-qm", "fix(x): implements XX-1\n\nbody.\n\nAmux-Session: lane-a")
    (wc / "f").write_text("2"); g("add", "-A")
    g("commit", "-qm", "fix(y): more XX-1 work\n\nAmux-Session: lane-a")
    (wc / "f").write_text("3"); g("add", "-A")
    g("commit", "-qm", "docs: note about XX-1\n\nAmux-Session: lane-b")
    (wc / "f").write_text("4"); g("add", "-A")
    g("commit", "-qm", "unrelated\n\nAmux-Session: lane-c")
    return str(wc)


def test_counts_by_session_most_common_first(srv, repo):
    srv._session_work_dir = lambda s: repo
    note = srv._card_citing_note("XX-1", "any")
    assert "lane-a (2)" in note and "lane-b (1)" in note, note
    assert "lane-c" not in note, "an uncited session leaked into the note"
    assert note.index("lane-a") < note.index("lane-b"), "not most-common-first"
    assert "advisory only" in note, (
        "the note stopped saying it is advisory — AC-302's whole design is "
        "that this text claims no authority")


def test_uncited_card_says_so_with_the_repo_named(srv, repo):
    """Revised per amux-cloud's residual on AMUX-2578: absence had three
    indistinguishable causes because the lookup runs in the ACKER's workdir —
    silence from the wrong repo read as "nobody committed against this". Now:
    RAN-and-found-nothing is a statement naming the repo it searched;
    could-not-run stays silent (next test). The wording still disclaims
    authority — an uncited fix is invisible, so this cannot be read as proof
    of no work."""
    srv._session_work_dir = lambda s: repo
    note = srv._card_citing_note("XX-404", "any")
    assert "no commits cite XX-404" in note, note
    assert "advisory only" in note


def test_errors_yield_absence_never_breakage(srv):
    """An advisory must never be the reason a transition fails."""
    srv._session_work_dir = lambda s: "/nonexistent/path"
    assert srv._card_citing_note("XX-1", "any") == ""
    srv._session_work_dir = lambda s: ""
    assert srv._card_citing_note("XX-1", "any") == ""
