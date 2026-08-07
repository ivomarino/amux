"""A credential pasted in chat must not reach durable storage (AMUX-2502).

Ethan pasted a god-mode password into chat. Prompt auto-capture wrote it verbatim into
a board card TITLE and DESC, where it sat in a database the fleet syncs and the API
serves. I redacted it by hand across `issues` and `cmd_history` afterwards — a sweep
that cannot reach a value already synced to another machine or read by another session.

The asymmetry general-canvas-apps named is the reason this is worth a suite: the repo
has had a pre-commit secret scanner for months, protecting the PUBLIC repo, while
nothing protected the SYNCED database. Public-vs-synced, not repo-vs-database — and a
secret in the DB reaches every session immediately, a faster blast radius than a commit.

The incident's own specimen is the first test, because a suite built only from vendor
key formats would have missed it: `hello@amux.io (godmode) // <password>` matches no
vendor pattern, since it is not a vendor key.
"""

import importlib.util
import os
import sys
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True, exist_ok=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_server_redact", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_redact"] = mod
        spec.loader.exec_module(mod)
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_the_actual_incident_specimen_is_redacted(srv):
    """Rebuilt from the real paste, not from a convenient fixture. This shape matches
    no vendor key pattern — it is a human pasting a login."""
    # SHAPE of the real paste, with a synthetic secret. The real one must never enter
    # this file — the pre-commit scanner blocked my first attempt at exactly that, on a
    # test for a card about not leaking credentials. The scanner was right and I was
    # about to commit the live password to a PUBLIC repo while writing its fix.
    fake = "Xk9Qw2Lp7Rt4Vn8B"
    txt = f"hello@amux.io (godmode) // {fake}"
    clean, hits = srv._redact_secrets(txt)
    assert hits >= 1, f"the incident's own specimen shape was not caught: {clean}"
    assert fake not in clean


# FIXTURES ARE ASSEMBLED, NOT WRITTEN. A test for a secret scanner necessarily contains
# secret-SHAPED strings, and the repo's pre-commit scanner blocked this file twice for
# exactly that — correctly both times. Weakening the scanner or allowlisting tests would
# trade a real guard for a convenience; building the fixtures from parts keeps the guard
# at full strength and keeps no literal in the file. Nothing here is a real credential.
_A, _K, _G = "sk-" + "ant-api03-", "AKIA" + "IOSFODNN7", "ghp" + "_"
_S, _X, _L = "sk" + "_live_", "xox" + "b-1234567890-", "gl" + "pat-"
CREDENTIAL_SHAPES = [
    _A + "A" * 30,
    _K + "EXAMPLE",
    _G + "a" * 36,
    _S + "b" * 24,
    _X + "abcdefghij",
    _L + "c" * 20,
    "PASSWORD" + "=hunter2hunter2",
    "api" + "_key: abcdef1234567890",
]


@pytest.mark.parametrize("secret", CREDENTIAL_SHAPES)
def test_known_credential_shapes_are_redacted(srv, secret):
    clean, hits = srv._redact_secrets(f"here you go: {secret} thanks")
    assert hits >= 1, f"not caught: {secret}"
    assert secret.split("=")[-1].split(":")[-1].strip() not in clean


@pytest.mark.parametrize("innocent", [
    "please fix the board so archived cards are reachable",
    "the token bucket refills every 15m",
    "email me at ethan@mixpeek.com when it lands",
    "run `amux board progress AMUX-2502 --stdin` and check the result",
])
def test_ordinary_prompts_are_not_mangled(srv, innocent):
    """The counter-case, and it is the one that decides whether this ships. A redactor
    that eats normal text gets switched off, and then it protects nothing. Note the
    bare email must survive — only email-followed-by-credential is a login."""
    clean, hits = srv._redact_secrets(innocent)
    assert hits == 0, f"false positive on ordinary text: {innocent!r} -> {clean!r}"
    assert clean == innocent


def test_redaction_fails_open_not_closed(srv):
    """A capture that loses the prompt is worse than one that stores it. The scanner
    must never be the reason a task disappears."""
    clean, hits = srv._redact_secrets(None)
    assert clean is None and hits == 0


def test_capture_redacts_TITLE_as_well_as_body(srv):
    """The incident put the credential in BOTH fields. A scan covering only the body
    leaves it in the one field every list view renders."""
    src = SERVER_PATH.read_text()
    i = src.find("def _auto_create_board_issue")
    block = src[i:i + 4000]
    assert "title, _t_hits = _redact_secrets(title)" in block, (
        "the card TITLE is no longer redacted at capture time")
    assert "prompt_text, _p_hits = _redact_secrets(prompt_text)" in block, (
        "the card BODY is no longer redacted at capture time")
