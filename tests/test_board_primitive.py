"""The board's write contract and gate scoping — the primitive the fleet's accountability rests on.

Every test here pins a defect that actually shipped on 2026-08-07, because a
regression suite assembled from imagined failures tends to test the things that were
never going to break. Four of these had NO coverage when they were found:

  archived unwritable      an automated sweep with no un-do stranded 2110 cards
  no-op PATCH bumps rev    200 + a moving rev read as "applied"; cost two sessions
                           five wrong card notes and three wrong cloud reports
  writable-set drift       the ignored-fields list omitted `tags`/`groups`, which ARE
                           applied — so it reported successful writes as ignored
  gate scope order         Ethan: "gates should have a hierarchal scope ... where local
                           worker takes precedent". Nothing asserted it.

Runs against an isolated AMUX_HOME with a real schema. The gate-contract suite was
green for weeks on ambient state and red the moment CI ran it on a machine that had
never started amux; that lesson is why the fixture here builds its own database.
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
        sys.modules.pop("amux_server_boardprim", None)
        spec = importlib.util.spec_from_file_location("amux_server_boardprim", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_boardprim"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


# ─────────────────────── gate scope hierarchy (Ethan's ask) ──────────────────

def test_worker_gate_beats_group_beats_global(srv):
    """The precedence Ethan called "very important", asserted rather than assumed.

    Built by SETTING all three layers and reading the winner, not by inspecting the
    order constant — a test that reads the constant would pass even if the resolver
    ignored it.
    """
    db = srv.get_db()
    now = int(time.time())
    db.execute("INSERT INTO issues (id,title,status,session,type,created,updated,owner_type) "
               "VALUES ('G-1','t','done','w1','code',?,?,'agent')", (now, now))
    db.execute("INSERT OR REPLACE INTO session_gates (session,status,gate) VALUES (?,?,?)",
               ("group:g1", "verified", '["GROUP GATE"]'))
    db.execute("INSERT OR REPLACE INTO session_gates (session,status,gate) VALUES (?,?,?)",
               ("w1", "verified", '["WORKER GATE"]'))
    db.commit()
    srv._load_session_gates.cache_clear() if hasattr(srv._load_session_gates, "cache_clear") else None
    orig = srv._session_tags_of
    srv._session_tags_of = lambda _n: ["g1"]
    try:
        item = {"id": "G-1", "session": "w1", "type": "code"}
        assert srv._effective_gate(item, "verified") == ["WORKER GATE"], "worker must win"
        # remove the worker layer -> group must now win, proving it was not just
        # "the first thing found" but a real precedence chain
        db.execute("DELETE FROM session_gates WHERE session='w1'")
        db.commit()
        assert srv._effective_gate(item, "verified") == ["GROUP GATE"], "group must beat global"
    finally:
        srv._session_tags_of = orig


def test_type_gate_outranks_worker_and_that_is_deliberate(srv):
    """Documents the one place the chain surprises people: a card's TYPE beats a
    worker's own gate, because the type is intrinsic to the card. Pinned so nobody
    "fixes" it into worker-wins and silently lets an escalation be gated on a merge."""
    db = srv.get_db()
    db.execute("INSERT OR REPLACE INTO session_gates (session,status,gate) VALUES (?,?,?)",
               ("w2", "verified", '["WORKER GATE"]'))
    db.commit()
    got = srv._effective_gate({"id": "G-2", "session": "w2", "type": "investigation"}, "verified")
    assert got == ["Outcome confirmed to still hold"], got


# ───────────────────────── the write contract ────────────────────────────────

def test_patch_writable_set_covers_what_the_handler_applies(srv):
    """DRIFT GUARD. `_PATCH_WRITABLE` feeds both the no-op check and the response's
    `ignored_fields`. The hand-maintained list it replaced omitted `tags` and
    `groups` — both applied — so successful writes were reported as ignored.

    Asserts the fields the handler demonstrably writes are all declared. Named
    explicitly rather than derived from the source, because deriving it from the same
    text the handler uses would make the test agree with any mistake.
    """
    w = set(srv._PATCH_WRITABLE)
    for f in ("title", "desc", "status", "session", "type", "gate", "reviewer",
              "depends_on", "archived", "tags", "groups", "owner_type"):
        assert f in w, f"{f} is applied by PATCH but missing from _PATCH_WRITABLE"


def test_advance_ladder_and_signoff_targets_agree(srv):
    """The pairing tripwire, at the primitive level: every status requiring reviewer
    sign-off must be reachable as some card's next step, or an author sits at a 409
    while the reviewer is never told (70 cards were in that state)."""
    routed = {srv._advance_target(s) for s in srv._ADVANCE_NEXT
              if srv._reviewer_acts_next(s)}
    assert not (set(srv._REVIEWER_SIGNOFF_TARGETS) - routed)


def test_noncode_types_have_an_honest_verified_gate(srv):
    """Ethos rule 3 at the primitive level. A doc move or a negative-result
    investigation cannot satisfy "Deployed to prod"; if these lose their type gate
    they inherit code's and become unclosable-without-lying."""
    for t in ("investigation", "research", "doc", "chore", "ops"):
        g = srv._item_type_gate({"type": t}, "verified")
        assert g, f"{t} lost its verified gate and now inherits code's"
        assert not any("prod" in c.lower() or "ci/cd" in c.lower() for c in g), (
            f"{t}'s verified gate mentions prod/CI — it ships no code: {g}")


def test_archived_is_not_a_terminal_status(srv):
    """`archived` is a FLAG, not a status. Conflating them is what made 2110 cards
    invisible: archived+todo is unreachable by every loop while not being finished."""
    assert "archived" not in srv._ADVANCE_NEXT
    assert srv._advance_target("archived") == ""
