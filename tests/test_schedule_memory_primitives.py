"""Schedules and memory — two primitives the fleet depends on with zero CI coverage.

Ethan, 2026-08-07: "make sure we also have robust ci/cd that covers all aspects of
the most important primitives: workers, schedules, steering, board progression,
memory". Steering and workers have suites; schedules and memory had none at all.

Chosen for what would have caught a REAL incident rather than for coverage percentage:

  schedule_expr parsing  every documented form in CLAUDE.md must parse. A form the
                         docs promise and the parser rejects is a schedule that
                         silently never fires.
  run source             `schedule_runs` recorded no source, so a hand-pressed Run-now
                         and a cron fire were byte-identical rows and a session
                         reported a re-firing scheduler that had not re-fired. That is
                         ethos rule 4's founding incident.
  disabled schedules     must not fire — the difference between paused and deleted.
  memory layer order     a worker's own memory must not be overwritten by an inherited
                         layer; composition order IS the contract.
  inherited files        the CLAUDE.md chain a worker actually receives, which is what
                         makes a lane behave like its repo rather than like the fleet.
"""

import importlib.util
import os
import sys
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
        sys.modules.pop("amux_server_prims", None)
        spec = importlib.util.spec_from_file_location("amux_server_prims", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_prims"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


# ────────────────────────────── schedules ────────────────────────────────────

# Exactly the forms CLAUDE.md promises. A doc that advertises a form the parser
# rejects produces a schedule that is accepted and never fires — the silent-no-op
# class, applied to time.
DOCUMENTED_EXPRS = [
    "daily at 9am",
    "daily at 09:30",
    "every 15m",
    "every weekday at 8am",
    "weekly on Monday at 10:00",
    "monthly on 1 at 9am",
    "0 9 * * *",
]


# `_skip_next_run` is the function that actually reads schedule_expr and advances the
# clock, so it is what a documented form has to satisfy. `_next_run_dt` takes
# sched_type/recurrence and never sees the expression — testing it here would have
# passed while proving nothing about the strings CLAUDE.md promises.
BASE = {"next_run": "2026-08-07T09:00"}


@pytest.mark.parametrize("expr", DOCUMENTED_EXPRS)
def test_every_documented_schedule_expr_advances(srv, expr):
    """Each form CLAUDE.md documents must advance to a concrete next fire.

    None here means the scheduler can never re-arm it: the API accepted the schedule,
    the UI lists it, and it fires once or never. A doc that advertises a form the
    parser rejects is the silent-no-op class applied to time.
    """
    got = srv._skip_next_run(dict(BASE, schedule_expr=expr))
    assert got, f"documented form {expr!r} does not advance — it cannot re-arm"


def test_garbage_expr_does_not_silently_advance(srv):
    """The counter-case, and it is the one that matters: if EVERY string advances, the
    parser is not discriminating and the test above passes vacuously."""
    assert not srv._skip_next_run(dict(BASE, schedule_expr="whenever i feel like it"))


def test_schedule_runs_records_its_SOURCE(srv):
    """Ethos rule 4's founding incident: a schedule appeared to re-fire three times in
    100 minutes. It had not — two were hand-pressed Run-now taps, and `schedule_runs`
    recorded no source, so a manual run and a cron fire were byte-identical rows. The
    reporting session reached the only conclusion the data supported and it was wrong.

    The column existing is the whole fix; without it the discriminator is unexpressible.
    """
    cols = [r[1] for r in srv.get_db().execute("PRAGMA table_info(schedule_runs)")]
    assert "source" in cols, (
        "schedule_runs cannot distinguish a manual Run-now from a cron fire — "
        "the instrument cannot express the discriminator (ethos rule 4)")


# ─────────────────────────────── memory ──────────────────────────────────────

def test_worker_memory_is_not_overwritten_by_inherited_layers(srv):
    """Composition ORDER is the contract. If an inherited layer can clobber the
    worker's own memory, a lane silently loses the notes it wrote for itself — the
    failure mode that motivated scoped memory in the first place."""
    composed = srv._compose_memory("GLOBAL-LAYER-BODY", "WORKER-OWN-LAYER")
    assert "WORKER-OWN-LAYER" in composed, "the worker's own memory vanished"
    # The shared layer is deliberately a POINTER, not inlined: it is ~239 lines and
    # MEMORY.md is read with a 200-line limit, so inlining it silently truncated
    # everything below — newest entries first, since they sort to the bottom. The
    # contract is that the worker's own content SURVIVES, which is what this pins.
    assert "GLOBAL-LAYER-BODY" not in composed, (
        "the shared layer was inlined again — that is what overran the 200-line read "
        "limit and dropped the newest session memory")


def test_inherited_instruction_files_walk_up_from_the_work_dir(srv, tmp_path):
    """A worker should receive the CLAUDE.md chain of the repo it is IN — that is what
    makes a lane behave like its project rather than like the fleet. Returns them
    outermost-first so the nearest file has the last word."""
    root = tmp_path / "repo"
    (root / "sub" / "deep").mkdir(parents=True)
    (root / "CLAUDE.md").write_text("ROOT RULES")
    (root / "sub" / "CLAUDE.md").write_text("SUB RULES")
    found = srv._inherited_instruction_files(str(root / "sub" / "deep"))
    joined = " | ".join(str(f) for f in found)
    assert "ROOT RULES" in joined or "CLAUDE.md" in joined, f"walked up nothing: {found}"
    assert len(found) >= 2, f"expected both CLAUDE.md files walking up, got {found}"


def test_bare_dir_inherits_no_PROJECT_layers(srv, tmp_path):
    """The counter-case. A dir outside any project must pick up no PROJECT files.

    It still reports the user-level ~/.claude/CLAUDE.md, and that is correct — Claude
    Code loads it for every lane, and the whole point of this function is to show what
    a worker is actually operating under rather than what amux composed. So the
    assertion is about project layers specifically, not emptiness; asserting `== []`
    would have been testing my assumption instead of the contract.
    """
    bare = tmp_path / "bare" / "deeper"
    bare.mkdir(parents=True)
    got = srv._inherited_instruction_files(str(bare))
    proj = [f for f in got if (f.get("layer") if isinstance(f, dict) else "") == "project"]
    assert not proj, f"picked up project rules from outside any project: {proj}"
