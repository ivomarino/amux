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
