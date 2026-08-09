"""The shared-checkout guard must not assert an INFERRED repo path as fact (AF-23).

The guard resolves its target repo from `git -C <path>` when present, and otherwise from
the session's cwd. It cannot see a `cd` inside a compound command — so a command that cd's
into a throwaway clone and commits THERE was refused as though it targeted the shared
checkout, naming by absolute path a repo the command never touched. Hit 2026-08-09.

The refusal read as a TRUE positive because it stated the inference as fact, which is the
part worth fixing: `-C` already resolved, so a precise escape existed and the message simply
never named it. An escape nobody is told about is not an escape (ethos rule 6).

Parsing `cd` was rejected deliberately and these tests encode that asymmetry. A wrong parse
fails OPEN on a real cross-session sweep — the entire thing this guard exists to catch —
so the third test below is the load-bearing one: the disclosure must not have widened `-C`
into a bypass.

These run against the GENERATOR (`_GIT_SHARED_GUARD_PY` in amux-server.py), not the
installed copy at ~/.amux/hooks/. _install_destroy_guard() overwrites a hand-edited install
on every restart, so the installed file is output; testing it would pin the wrong artifact.
"""
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

SERVER_PATH = str(Path(__file__).parent.parent / "amux-server.py")


@pytest.fixture(scope="module")
def guard(tmp_path_factory):
    """Extract the generated guard to a real file and run it as a real subprocess.

    Deliberately end-to-end via stdin JSON rather than importing its functions: the defect
    was in the RENDERED refusal text, and a test that called an internal predicate would
    have passed against the broken version.
    """
    d = tmp_path_factory.mktemp("guard")
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True, exist_ok=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_guard_src", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_guard_src"] = mod
        spec.loader.exec_module(mod)
        p = d / "git-shared-guard.py"
        p.write_text(mod._GIT_SHARED_GUARD_PY)
        return str(p)
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


SHARED = "/Users/ethan/Dev/amux"
NOTE = "Repo INFERRED from this session's working directory"


def _run(guard, cmd, cwd):
    r = subprocess.run([sys.executable, guard],
                       input=json.dumps({"tool_name": "Bash", "cwd": cwd,
                                         "tool_input": {"command": cmd}}),
                       capture_output=True, text=True, timeout=60)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def test_a_cwd_inferred_refusal_discloses_that_it_inferred(guard):
    """THE FIX. Still blocks — the point was never to allow this — but the message now
    says the repo was inferred and names `git -C` as the way to target another one."""
    rc, out = _run(guard, "git commit -am wip", SHARED)
    assert rc == 2, "the guard stopped blocking a bare `commit -a` on the shared tree"
    assert NOTE in out, (
        "the refusal still asserts the inferred repo path as fact (AF-23); output:\n%s" % out)
    assert "git -C" in out, "the refusal does not name the escape that already works"


def test_an_explicit_dash_C_at_the_shared_repo_STILL_BLOCKS(guard):
    """THE LOAD-BEARING ONE. The disclosure tells people to use `git -C`. If that had
    widened into a bypass, this fix would have converted a false positive into a false
    NEGATIVE — trading one wasted retry for another session's uncommitted work.

    Note this also pins the negative half of the disclosure: with `-C` explicit there is
    no inference, so the note must be ABSENT. Without that assertion the test would pass
    against a version that printed the note unconditionally, which would make it noise
    everywhere and get it ignored where it matters."""
    rc, out = _run(guard, "git -C %s commit -am wip" % SHARED, "/tmp")
    assert rc == 2, (
        "`git -C <shared repo> commit -am` was ALLOWED — the disclosure turned the "
        "escape into a bypass (AF-23)")
    assert NOTE not in out, (
        "the guard claimed it inferred the repo when `-C` named it explicitly — nothing "
        "was inferred, so disclosing an inference is a lie in the other direction")


def test_the_escape_actually_works_for_a_scratch_repo(guard):
    """The counter-case that makes the advice honest. If `-C` at a non-shared path were
    still refused, the refusal would be routing people to a door that does not open —
    which is the failure mode ethos rule 6 is about (AMUX-2325: a sanctioned escape that
    cannot be walked gets walked from an unaudited path instead)."""
    rc, out = _run(guard, "git -C /tmp/scratch-clone commit -am wip", SHARED)
    assert rc == 0, (
        "a commit aimed at a scratch repo was still blocked, so the escape the refusal "
        "advertises does not work; output:\n%s" % out)


def test_the_guard_fails_open_on_a_malformed_payload(guard):
    """A guard bug must never break tool calls — the generator's own closing contract
    (`except Exception: sys.exit(0)`). Pinned because a disclosure string built from an
    unset variable would raise, and failing CLOSED here would wedge every Bash call in
    every session on the machine."""
    r = subprocess.run([sys.executable, guard], input="not json at all",
                       capture_output=True, text=True, timeout=60)
    assert r.returncode == 0, "the guard failed closed on malformed input (rc=%d)" % r.returncode
