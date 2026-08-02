"""Unit tests for amux session resume — the path that decides whether a
restarting session resumes its conversation or wakes up blank.

Three defects made this dead code on every install. These tests pin all three:
the title lives on line 2 and the lookup only read line 1; the lookup demanded
a unique name match while every fresh start added another identically-named
conversation; and the name was only persisted on graceful stop.

Loaded via importlib like tests/test_shell_quote_flags.py so no drift is possible.
"""

import importlib.util
import json
import os
import sys
import time
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SERVER_PATH = REPO_ROOT / "amux-server.py"


@pytest.fixture(scope="module")
def amux_server():
    spec = importlib.util.spec_from_file_location("amux_server", SERVER_PATH)
    assert spec is not None and spec.loader is not None, f"could not load {SERVER_PATH}"
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_server"] = mod
    spec.loader.exec_module(mod)
    return mod


def _write_jsonl(path: Path, entries):
    path.write_text("\n".join(json.dumps(e) for e in entries) + "\n")


def _header(title):
    """The header block Claude Code writes: custom-title is NOT line 1."""
    return [
        {"type": "last-prompt", "leafUuid": "abc"},
        {"type": "custom-title", "customTitle": title, "sessionId": "x"},
    ]


def _msg(role="user"):
    return {"type": role, "message": {"role": role, "content": "hi"}}


# ── _cc_session_title ────────────────────────────────────────────────────────

def test_title_on_line_two_is_found(amux_server, tmp_path):
    """The regression: Claude Code writes custom-title on line 2, and the old
    lookup read only line 1, so it matched nothing on any install."""
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, _header("Amux-gtm") + [_msg()])
    assert amux_server._cc_session_title(f) == "Amux-gtm"


def test_title_on_line_one_is_found(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, [{"type": "custom-title", "customTitle": "Solo"}])
    assert amux_server._cc_session_title(f) == "Solo"


def test_session_name_key_is_accepted(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, [{"type": "meta"}, {"sessionName": "Legacy"}])
    assert amux_server._cc_session_title(f) == "Legacy"


def test_title_absent_returns_empty(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, [_msg(), _msg("assistant")])
    assert amux_server._cc_session_title(f) == ""


def test_title_scan_is_bounded(amux_server, tmp_path):
    """A title past the scan window is not found — the read must stay bounded
    because this runs on every session-list refresh."""
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, [_msg()] * 40 + [{"type": "custom-title", "customTitle": "TooLate"}])
    assert amux_server._cc_session_title(f, max_lines=30) == ""


def test_malformed_lines_are_skipped(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    f.write_text("not json\n" + json.dumps({"customTitle": "Survivor"}) + "\n")
    assert amux_server._cc_session_title(f) == "Survivor"


def test_missing_file_returns_empty(amux_server, tmp_path):
    assert amux_server._cc_session_title(tmp_path / "nope.jsonl") == ""


def test_non_dict_json_line_is_skipped(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    f.write_text('["a list"]\n' + json.dumps({"customTitle": "Survivor"}) + "\n")
    assert amux_server._cc_session_title(f) == "Survivor"


# ── _jsonl_has_messages ─────────────────────────────────────────────────────

def test_has_messages_true_for_real_conversation(amux_server, tmp_path):
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, _header("X") + [_msg("assistant")])
    assert amux_server._jsonl_has_messages(f) is True


def test_has_messages_false_for_snapshot_only(amux_server, tmp_path):
    """claude --resume exits instantly on these, so resuming one is a fresh
    start with extra steps."""
    f = tmp_path / "conv.jsonl"
    _write_jsonl(f, _header("X") + [{"type": "file-history-snapshot"}])
    assert amux_server._jsonl_has_messages(f) is False


def test_has_messages_false_for_missing_file(amux_server, tmp_path):
    assert amux_server._jsonl_has_messages(tmp_path / "nope.jsonl") is False


# ── name → conversation id ──────────────────────────────────────────────────

@pytest.fixture
def project(amux_server, tmp_path, monkeypatch):
    """Build a fake ~/.claude/projects/<slug>/ and return a writer + work_dir."""
    monkeypatch.setattr(amux_server, "CLAUDE_HOME", tmp_path)
    work_dir = "/Users/someone/Projects/demo"
    proj = tmp_path / "projects" / amux_server._project_name(work_dir)
    proj.mkdir(parents=True)

    def add(uuid, title, mtime, with_messages=True):
        f = proj / f"{uuid}.jsonl"
        entries = _header(title) + ([_msg()] if with_messages else
                                    [{"type": "file-history-snapshot"}])
        _write_jsonl(f, entries)
        os.utime(f, (mtime, mtime))
        return f

    return add, work_dir


def test_resolves_title_recorded_on_line_two(amux_server, project):
    """End-to-end for the line-1 bug: nothing resolved before this."""
    add, work_dir = project
    add("aaaaaaaa-0000-0000-0000-000000000000", "Amux-gtm", time.time())
    assert amux_server._cc_session_id_for_name("Amux-gtm", work_dir) == \
        "aaaaaaaa-0000-0000-0000-000000000000"


def test_many_same_named_sessions_resolve_to_newest(amux_server, project):
    """The death spiral: each fresh start added another 'Amux-gtm' conversation,
    and the old code required exactly one match — so once it had failed twice it
    could never succeed again. Newest wins instead."""
    add, work_dir = project
    now = time.time()
    add("11111111-0000-0000-0000-000000000000", "Amux-gtm", now - 500_000)
    add("22222222-0000-0000-0000-000000000000", "Amux-gtm", now - 100_000)
    add("33333333-0000-0000-0000-000000000000", "Amux-gtm", now - 10)
    add("44444444-0000-0000-0000-000000000000", "Amux-gtm", now - 200_000)
    assert amux_server._cc_session_id_for_name("Amux-gtm", work_dir) == \
        "33333333-0000-0000-0000-000000000000"


def test_snapshot_only_files_are_not_resume_targets(amux_server, project):
    """A newer snapshot-only file must not beat an older real conversation."""
    add, work_dir = project
    now = time.time()
    add("aaaaaaaa-0000-0000-0000-000000000000", "Amux-gtm", now - 1000)
    add("bbbbbbbb-0000-0000-0000-000000000000", "Amux-gtm", now, with_messages=False)
    assert amux_server._cc_session_id_for_name("Amux-gtm", work_dir) == \
        "aaaaaaaa-0000-0000-0000-000000000000"


def test_no_match_returns_empty(amux_server, project):
    add, work_dir = project
    add("aaaaaaaa-0000-0000-0000-000000000000", "Other-Session", time.time())
    assert amux_server._cc_session_id_for_name("Amux-gtm", work_dir) == ""


def test_missing_project_dir_returns_empty(amux_server, tmp_path, monkeypatch):
    monkeypatch.setattr(amux_server, "CLAUDE_HOME", tmp_path)
    assert amux_server._cc_session_id_for_name("Amux-gtm", "/no/such/dir") == ""


def test_exists_in_project_sees_line_two_title(amux_server, project):
    add, work_dir = project
    add("aaaaaaaa-0000-0000-0000-000000000000", "Amux-gtm", time.time())
    assert amux_server._cc_session_exists_in_project("Amux-gtm", work_dir) is True


def test_exists_in_project_false_when_absent(amux_server, project):
    add, work_dir = project
    add("aaaaaaaa-0000-0000-0000-000000000000", "Other", time.time())
    assert amux_server._cc_session_exists_in_project("Amux-gtm", work_dir) is False


def test_candidates_are_ordered_newest_first(amux_server, project):
    add, work_dir = project
    now = time.time()
    add("11111111-0000-0000-0000-000000000000", "S", now - 300)
    add("22222222-0000-0000-0000-000000000000", "S", now - 100)
    got = [p.stem[:8] for p in amux_server._cc_session_candidates("S", work_dir)]
    assert got == ["22222222", "11111111"]


def test_candidates_empty_when_project_name_resolution_raises(amux_server, tmp_path, monkeypatch):
    """A pathological work_dir (e.g. a symlink cycle, or a home directory that
    can't be determined) can raise RuntimeError out of Path.expanduser()/
    resolve() inside _project_name. That must not escape into session startup."""
    monkeypatch.setattr(amux_server, "CLAUDE_HOME", tmp_path)

    def boom(work_dir):
        raise RuntimeError("symlink cycle")

    monkeypatch.setattr(amux_server, "_project_name", boom)
    assert amux_server._cc_session_candidates("Amux-gtm", "/some/dir") == []


def test_candidates_empty_when_project_dir_cannot_be_listed(amux_server, project, monkeypatch):
    """A project directory that exists but raises OSError on iteration (e.g.
    permission denied) must not raise into session startup. Forced via
    monkeypatch rather than chmod so this passes when run as root too."""
    add, work_dir = project
    add("aaaaaaaa-0000-0000-0000-000000000000", "Amux-gtm", time.time())

    def boom(self, pattern):
        raise OSError("permission denied")

    monkeypatch.setattr(Path, "glob", boom)
    assert amux_server._cc_session_candidates("Amux-gtm", work_dir) == []
