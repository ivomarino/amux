"""What SURVIVED the worker-card counts row, plus the board defects it exposed.

Ethan asked for a colour-coded per-status count on each worker card, then — after
seeing it on a phone — asked for the whole row to be removed. It is gone (the chips,
the total, the schedule chip, the server-side issue_counts payload and its query).
The eleven tests that covered it went with it: a test whose subject is deleted is
not a regression guard, it is a tripwire on a ghost.

One of those eleven had already gone false-green before it was removed, which is the
reason this note exists rather than a silent deletion. `test_card_renders_the_
schedule_count_from_ONE_source` asserted `"sched_on" in _renderSessionCard`, and
after the row was deleted it kept passing — on a leftover COMMENT that still
mentioned s.sched_on. The renderer had no schedule count at all. A source-reading
test cannot tell code from prose unless it is written to, and this one was not.

What remains here is everything the removal did NOT invalidate, all of it found
while building the row rather than being about the row:

  worker: was never a key    `_bqMatch` has a `case 'worker'` arm with a comment
                             explaining the aliasing, the search placeholder says
                             `-worker:none`, and the filter menu emits `worker:<name>`
                             — but `worker` was absent from `_BQ_KEYS`, so `_bqParse`
                             dropped the token to free text and the query matched
                             NOTHING. Three surfaces documenting a filter that returns
                             an empty board, and an arm that could never execute.

  group rendered twice       s.tags appeared as a grp-chip beside the worker name AND
                             as a .tag badge below, with two different click actions.
                             Still fixed; the row's removal does not touch it.

  stray board columns        A status with cards and no column was filed under To Do,
                             so 51 cards displayed as something they were not, and
                             the two board view modes disagreed about the same card.

The JS tests read the SHIPPED source rather than a paraphrase — the client lives in a
Python string literal, so `ast.parse` is blind to it and `node --check` proves only
that it parses. Neither can see a key missing from a list.
"""

import importlib.util
import os
import re
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"
SRC = SERVER_PATH.read_text()


@pytest.fixture
def srv(tmp_path):
    home = tmp_path / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_cardcounts", None)
        spec = importlib.util.spec_from_file_location("amux_server_cardcounts", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_cardcounts"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        mod._AMUX_TEST_HOME = home
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def _js_block(name):
    """The body of a client function, by brace matching from its declaration.

    Byte windows are the wrong anchor — two earlier source guards in this repo were
    wrong in OPPOSITE directions (one matched its own comment, one used a fixed +3000
    offset that stopped reaching the code it guarded). Matching braces tracks the
    function wherever it moves.
    """
    start = SRC.find("function %s(" % name)
    assert start > 0, "%s is gone from the client" % name
    i = SRC.index("{", start)
    depth, j = 0, i
    while j < len(SRC):
        if SRC[j] == "{":
            depth += 1
        elif SRC[j] == "}":
            depth -= 1
            if depth == 0:
                return SRC[i:j + 1]
        j += 1
    raise AssertionError("unbalanced braces in %s" % name)


# ───────────────── the dead filter key (worker:) ─────────────────────────────

def _bq_keys():
    m = re.search(r"const _BQ_KEYS = \[(.*?)\];", SRC, re.S)
    assert m, "_BQ_KEYS is gone"
    return {v.strip().strip("'\"") for v in m.group(1).split(",") if v.strip()}


def _bqmatch_case_labels():
    """Every `case 'x':` inside _bqMatch's term switch — i.e. every key the matcher
    believes it can resolve."""
    body = _js_block("_bqMatch")
    return set(re.findall(r"case\s+'([a-z_]+)'\s*:", body))


def test_every_key_the_matcher_handles_is_a_key_the_parser_ACCEPTS():
    """DRIFT GUARD, and the one that would have caught the shipped bug.

    `_bqParse` only emits a term when the key is in `_BQ_KEYS`. So a `case` arm for a
    key absent from that list is unreachable code, and — worse — the token it was
    written for silently degrades to a free-text search for the literal string
    "worker:amux", which matches no card. The board comes back empty and nothing
    reports an error.
    """
    keys, cases = _bq_keys(), _bqmatch_case_labels()
    assert cases, "found no case labels — the extractor broke, not the code"
    orphans = cases - keys
    assert not orphans, (
        "_bqMatch handles %s but _BQ_KEYS does not list it, so _bqParse drops the "
        "token to free text and the filter silently matches nothing" % sorted(orphans))
    print("checked %d matcher arms against %d parsed keys, all reachable"
          % (len(cases), len(keys)))


def test_the_orphan_detector_can_fail():
    """Prove the check above discriminates by seeding the exact historical state:
    remove `worker` from the key set and confirm it goes red. A drift guard that
    cannot go red is the thing it is guarding against."""
    seeded = _bq_keys() - {"worker"}
    assert _bqmatch_case_labels() - seeded == {"worker"}, (
        "dropping 'worker' from _BQ_KEYS no longer trips the comparison — the guard "
        "above cannot detect the defect it was written for")


def test_worker_is_documented_and_therefore_must_work():
    """The surfaces that TELL you to use `worker:`. Following an instruction exactly
    is what produced the failure here (the AMUX-2140 shape), so the instruction and
    the parser are asserted together rather than separately."""
    assert "-worker:none" in SRC, "the search placeholder no longer documents worker:"
    assert "'worker:' + v" in SRC, "the filter menu no longer emits worker: tokens"
    assert "worker" in _bq_keys(), (
        "two surfaces tell the user to type worker: and the parser does not know it")


# ───────────────── no duplicate render of one fact ───────────────────────────


def test_card_renders_each_group_ONCE():
    """`s.tags` is the worker's groups. It was rendered twice — as a `grp-chip` beside
    the name (scopes the dashboard to that group) and again as a `.tag` badge below
    (filters the list). Same text, same card, two different actions, which is worse
    than a plain duplicate because the second one teaches you the first is something
    else. Tag filtering is unaffected: the filter bar still offers every group.
    """
    body = _js_block("_renderSessionCard")
    renders = re.findall(r"(?:s\.tags\s*\|\|\s*\[\]|s\.tags)\s*\)?\s*\.map\(", body)
    assert len(renders) == 1, (
        "the worker card renders s.tags %d times; exactly one chip per group, or the "
        "card shows the same group twice under two behaviours" % len(renders))


def test_badges_guard_does_not_reserve_space_for_removed_tags():
    """The follow-on nobody would notice: the badges row's render condition still
    listing `s.tags.length` would emit an empty bordered strip for a worker that has
    groups and nothing else. Removing a render means removing it from its guard."""
    body = _js_block("_renderSessionCard")
    m = re.search(r"\$\{\(([^)]*)\) \? `<div class=\"badges\">", body)
    assert m, "the badges row moved — re-anchor this test"
    assert "s.tags" not in m.group(1), (
        "the badges row is still gated on s.tags.length but no longer renders tags, "
        "so a groups-only worker gets an empty badges row")


# ───────────────── the counts themselves ─────────────────────────────────────

def _mk_session(srv, name):
    (Path(srv._AMUX_TEST_HOME) / "sessions" / (name + ".env")).write_text("CC_DIR=/tmp\n")


def _mk_issue(srv, iid, session, status, archived=0):
    now = int(time.time())
    srv.get_db().execute(
        "INSERT INTO issues (id,title,status,session,type,created,updated,owner_type,archived) "
        "VALUES (?,?,?,?,'code',?,?,'agent',?)", (iid, iid, status, session, now, now, archived))
    srv.get_db().commit()


# ───────────────── the status letters must actually discriminate ─────────────


# The board's real columns, plus the statuses that live in the DB but have no
# column (43 cards sat in `blocked`/`needsyou` when this shipped). The extras are
# the harder case: they arrive one at a time, after the map is built.


# ───────────────── board columns must not mislabel strays (AMUX-2526) ────────

def test_column_mode_does_not_file_unknown_statuses_under_todo():
    """51 live cards displayed as To Do while carrying a different status.

    The bucketer's else-branch was `cols['todo'].push(item)`, so 26 needsyou, 17
    blocked, 6 archived, 1 armed and 1 resolved all rendered under To Do. The alias
    comment in this same file says a card reading "resolved" displayed as "To Do" is
    a lie — and the fallback told it, at scale, twelve lines below the comment.
    """
    body = _js_block("renderBoard")
    assert "strayCols" in body, (
        "the column bucketer no longer collects unknown statuses — check whether the "
        "todo fallback is back")
    assert not re.search(r"else\s*\{\s*cols\['todo'\]", body), (
        "unknown statuses are filed under To Do again: every card in a status with no "
        "column will display as To Do (AMUX-2526)")


def test_both_board_view_modes_share_the_stray_predicate():
    """The real defect was DISAGREEMENT, not just mislabelling. List mode already
    appended unknown statuses as their own group with the real name, so one card read
    "blocked" in list mode and "To Do" in column mode. Two views of one board
    disagreeing about one card is worse than either being wrong alone, because
    whichever you opened first is the one you believe.
    """
    body = _js_block("renderBoard")
    # list mode: appends keys not in its own order array
    assert re.search(r"order\.concat\(Object\.keys\(groups\)\.filter", body), (
        "list mode no longer appends unknown statuses as their own group")
    # column mode: appends strays after the configured columns
    assert re.search(r"boardStatuses\.concat\(strayCols", body), (
        "column mode no longer appends stray statuses, so it disagrees with list mode")


def test_stray_columns_do_not_offer_controls_that_cannot_work():
    """A stray column has no stored gate and was never configured, so editStatusGate
    would write a gate for a status absent from the list and deleteBoardStatus would
    delete a column that does not exist. A control that cannot do what it says is the
    `amux board claim` shape — the instruction succeeds and nothing happens."""
    body = _js_block("renderBoard")
    assert "stObj.stray" in body, "stray columns are no longer distinguished at render"
    assert re.search(r"if\s*\(!stObj\.stray\)\s*\{", body), (
        "the gate/delete controls are offered on stray columns again")


def test_stray_columns_are_labelled_as_unconfigured():
    """Ethos rule 8: what the board's statuses ARE is Ethan's decision. Surfacing the
    cards without silently promoting the status to a real column is the whole design
    — so the column has to SAY it is unconfigured, or it just looks like a status
    somebody added."""
    body = _js_block("renderBoard")
    # The flag must be emitted UNDER the stray guard, not merely present in the
    # file. The first version of this test asserted `"col-stray-flag" in body`
    # and passed against a seeded copy where the guard had been stubbed to
    # `if (false)` — the string was still there, unreachable. A test that green-
    # lights dead code is exactly the theatre this file keeps finding elsewhere,
    # and it took seeding the defect to notice, not reading the test.
    assert re.search(r"if\s*\(stObj\.stray\)\s*\{[^}]*col-stray-flag", body, re.S), (
        "the unconfigured marker is not emitted under the stray guard, so an "
        "unconfigured status is indistinguishable from a configured one")


def test_stray_column_counts_are_remembered_across_renders():
    """The follow-on: _prevCardRects drives the count bump animation. Snapshotting
    only boardStatuses leaves a stray column's `prev` permanently 0, so its counter
    re-animates on every render forever — a cosmetic bug that reads as live activity
    on a column that has not changed."""
    body = _js_block("renderBoard")
    assert re.search(r"_renderCols\.forEach\(stObj => \{ _prevCardRects", body), (
        "_prevCardRects is snapshotted from boardStatuses only, so stray columns bump "
        "on every render")
