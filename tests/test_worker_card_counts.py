"""The worker card's counts row, and the drift that let a filter key be dead code.

Ethan, 2026-08-07: "card view on list page has some duplicate stuff like scheduler
count and group id twice also it should have a count color coded for each count of
issues per status on the board".

Three things are pinned here, each because fixing his report surfaced a defect that
nothing could have caught:

  worker: was never a key    `_bqMatch` has a `case 'worker'` arm with a comment
                             explaining the aliasing, the search placeholder says
                             `-worker:none`, and the filter menu emits `worker:<name>`
                             — but `worker` was absent from `_BQ_KEYS`, so `_bqParse`
                             dropped the token to free text and the query matched
                             NOTHING. Three surfaces documenting a filter that returns
                             an empty board, and an arm that could never execute.

  two schedule counts        The card rendered s.sched_on/s.sched_off (server payload,
                             refreshed every poll) AND a count derived from the client
                             `schedules` array (refetched only on the scheduler view).
                             Not merely redundant: they drift, and then one card shows
                             two different answers to one question.

  issue_counts predicate     The chips tap through to the board. If the count and the
                             list you land on use different predicates, the number is a
                             view disagreeing with the mechanism it describes.

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

def test_card_renders_the_schedule_count_from_ONE_source():
    """The card must read schedule counts from the sessions payload only.

    Deriving a second count from the client `schedules` array is what put two numbers
    for one question on one card — and they are not equivalent: `schedules` is
    refetched on the scheduler view, `s.sched_on` on every sessions poll.
    """
    body = _js_block("_renderSessionCard")
    assert "sched_on" in body, "the card stopped showing schedule counts entirely"
    assert not re.search(r"schedules\s*\.\s*filter", body), (
        "the card derives a schedule count from the client `schedules` array again — "
        "that is the stale second opinion, not a second copy of the same number")


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


def test_issue_counts_are_grouped_by_status_per_worker(srv):
    """The payload the chips render. Computed server-side on purpose: the client's
    board array is only fetched when the board view is open, so a client-derived count
    reads 0 on a fresh load of the Workers tab."""
    _mk_session(srv, "w1")
    _mk_session(srv, "w2")
    _mk_issue(srv, "C-1", "w1", "todo")
    _mk_issue(srv, "C-2", "w1", "todo")
    _mk_issue(srv, "C-3", "w1", "doing")
    _mk_issue(srv, "C-4", "w2", "verified")
    by = {s["name"]: s.get("issue_counts") or {} for s in srv.list_sessions()}
    assert by["w1"] == {"todo": 2, "doing": 1}, by["w1"]
    assert by["w2"] == {"verified": 1}, by["w2"]


def test_archived_cards_are_excluded_from_the_counts(srv):
    """The chips tap through to the board's default view, which hides archived cards.
    A count that included them would send you to a shorter list than the number
    promised — a view disagreeing with the mechanism it describes (ethos rule 1), and
    the exact sign-error that greeted a lane with 199 queued items when 9 were real.
    """
    _mk_session(srv, "w3")
    _mk_issue(srv, "C-5", "w3", "todo")
    _mk_issue(srv, "C-6", "w3", "todo", archived=1)
    got = {s["name"]: s.get("issue_counts") or {} for s in srv.list_sessions()}["w3"]
    assert got == {"todo": 1}, (
        "archived cards are counted on the worker card but hidden on the board it "
        "links to: %s" % got)


def test_a_worker_with_no_cards_reports_an_empty_map_not_a_missing_key(srv):
    """The renderer does `(s.issue_counts || {})`, so a missing key is survivable —
    but the row's emptiness must come from having no cards, not from the field being
    absent, or "no issues" and "field not shipped" become indistinguishable."""
    _mk_session(srv, "w4")
    s = [x for x in srv.list_sessions() if x["name"] == "w4"][0]
    assert "issue_counts" in s, "issue_counts is missing from the sessions payload"
    assert s["issue_counts"] == {}


def test_counts_row_colours_come_from_statusStyle_not_a_second_palette():
    """One colour source. A second map keyed by status name would drift from the board
    columns and, worse, would not cover CUSTOM statuses at all — statusStyle() falls
    through to the custom palette, a hand-written map returns undefined and the chip
    renders unstyled."""
    body = _js_block("_cardCountsRow")
    assert "statusStyle(" in body, (
        "the counts row no longer uses statusStyle — colours will drift from the "
        "board columns and custom statuses will render unstyled")


def test_counts_row_orders_by_the_board_and_drops_nothing(srv):
    """Two properties of the ordering that are easy to get wrong in opposite ways:
    it must follow the board's own column order, and a status the board no longer
    lists must still be COUNTED rather than silently vanishing — hiding cards is the
    failure this row exists to end."""
    body = _js_block("_cardStatusCounts")
    assert "boardStatuses" in body, "the row no longer follows the board's column order"
    assert "known.includes" in body or "!known" in body, (
        "statuses absent from boardStatuses are dropped instead of appended, so cards "
        "carrying a retired status become invisible on the worker card")


# ───────────────── the status letters must actually discriminate ─────────────

def _run_abbr_js(statuses, extras):
    """Execute the SHIPPED _statusAbbrs/_statusAbbr in node against a given status
    set. Running the real functions rather than reimplementing them is the point:
    a mirror would agree with whatever mistake the source makes."""
    import json
    import subprocess
    js = (
        "const boardStatuses = %s;\n" % json.dumps(statuses)
        + "function _abbrOf(label, taken)%s\n" % _js_block("_abbrOf")
        + "function _statusAbbrs()%s\n" % _js_block("_statusAbbrs")
        + "function _statusAbbr(id, st)%s\n" % _js_block("_statusAbbr")
        + "const st = _statusAbbrs();\n"
        + "const out = {};\n"
        + "for (const s of boardStatuses) out[s.id] = _statusAbbr(s.id, st);\n"
        + "for (const e of %s) out[e] = _statusAbbr(e, st);\n" % json.dumps(extras)
        + "console.log(JSON.stringify(out));\n"
    )
    r = subprocess.run(["node", "-e", js], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    return json.loads(r.stdout)


# The board's real columns, plus the statuses that live in the DB but have no
# column (43 cards sat in `blocked`/`needsyou` when this shipped). The extras are
# the harder case: they arrive one at a time, after the map is built.
_STATUSES = [
    {"id": "backlog", "label": "Backlog"}, {"id": "todo", "label": "To Do"},
    {"id": "doing", "label": "In Progress"}, {"id": "review", "label": "In Review"},
    {"id": "done", "label": "Done"}, {"id": "verified", "label": "Verified"},
    {"id": "discarded", "label": "Discarded"},
]
_EXTRAS = ["blocked", "needsyou", "armed", "resolved"]


def test_every_status_gets_a_DISTINCT_letter():
    """The whole reason the letter exists. Colour does not separate todo from
    discarded (both grey) or backlog from review from blocked (all blue), so if two
    statuses also share a letter the chip is undecodable and the biggest number on
    the card — usually discarded — reads as the queue."""
    got = _run_abbr_js(_STATUSES, _EXTRAS)
    letters = list(got.values())
    dupes = {l for l in letters if letters.count(l) > 1}
    assert not dupes, "statuses share a letter %s: %s" % (sorted(dupes), got)
    print("checked %d statuses, all letters distinct: %s" % (len(got), got))


def test_done_and_discarded_do_not_collide():
    """Named explicitly because it is the pair a naive first-letter scheme gets
    wrong, and the pair whose confusion is most expensive: 106 done and 82 discarded
    on one card, one meaning shipped and one meaning abandoned."""
    got = _run_abbr_js(_STATUSES, _EXTRAS)
    assert got["done"] != got["discarded"], got
    assert got["done"] == "D", "Done should keep the plain initial: %s" % got["done"]


def test_multi_word_labels_use_word_INITIALS():
    """"In Progress" and "In Review" both start with I, so a first-letter rule
    shadows one of them. Word initials give IP and IR, which is also how people
    already write those statuses."""
    got = _run_abbr_js(_STATUSES, _EXTRAS)
    assert got["doing"] == "IP", "In Progress should read IP: %s" % got["doing"]
    assert got["review"] == "IR", "In Review should read IR: %s" % got["review"]
    assert got["todo"] == "TD", "To Do should read TD: %s" % got["todo"]


def test_the_LAST_WORD_rule_would_have_broken_todo():
    """Pins WHY the rule is word-initials and not "last word", because last-word is
    the obvious simplification someone will reach for and it shipped here first:
    "To Do" -> last word "Do" -> D, which takes Done's letter and then READS as
    Done. The uniqueness test caught it; this one records the reason so the fix is
    not undone by a refactor that looks tidier."""
    got = _run_abbr_js(_STATUSES, _EXTRAS)
    assert got["todo"] != got["done"], (
        "To Do and Done share a letter — the last-word rule is back: %s" % got)


def test_a_status_with_no_column_still_gets_a_unique_letter():
    """Extras are resolved lazily, after the map exists. If they collided with an
    existing letter the row would show two chips reading the same thing — worse than
    the colour collision it replaced, because a letter looks authoritative."""
    got = _run_abbr_js(_STATUSES, _EXTRAS)
    known = {got[s["id"]] for s in _STATUSES}
    for e in _EXTRAS:
        assert got[e] not in known, "%s took an existing status's letter %r" % (e, got[e])
        known.add(got[e])


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
