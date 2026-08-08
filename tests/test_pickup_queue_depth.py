"""Auto-pickup must tell a lane how deep its queue is (AMUX-2533).

Reported by mixpeek-studio while ~90 restored personas todos were being fed to them
ONE CARD AT A TIME. The pickup notice described a single card and never the queue, so
a lane taking card 1 of 90 could not know there were 89 behind it: scope, decide, go
idle, get the next, repeat — 90 full cold-cache turns. Theirs was routed into the most
expensive lane in the fleet, already flagged at 543k tokens/turn.

The fix is INFORMATION, not an exemption. A "skip expensive lanes" rule would make
cards silently undispatchable with nothing saying so — the ethos rule-1 trap where an
exclusion does not make something cheap, it makes it invisible. Depth and age are cheap,
visible in the notice, and leave judgement with the lane: one that knows it is 1-of-90
can batch, triage, or say the queue is mis-shaped. One told nothing can only grind.

The advance nudge has said "N more card(s) queued" for a while. Pickup never did — the
same information, one subsystem over, never carried across. That is the third instance
this week of a remedy existing and something upstream not reaching it.
"""
import re
from pathlib import Path

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _pickup_block():
    i = SRC.find("[amux auto-pickup] Claimed board card")
    assert i > 0, "the pickup notice moved — re-anchor this test"
    # Bound the START at the claim/commit that precedes it, and the END at the
    # notice itself. Both boundaries deliberate: a positional probe on a 76k-line
    # file finds neighbours otherwise (amux-cloud, 2026-08-08).
    j = SRC.rfind("_append_board_log(item_id", 0, i)
    assert j > 0
    return SRC[j:i + 1200]


def test_pickup_counts_the_rest_of_the_queue():
    """The regression: the notice must state how many cards are queued behind this one."""
    b = _pickup_block()
    assert "more card(s) are queued behind this one" in b, (
        "the pickup notice no longer tells the lane its queue depth, so a lane taking "
        "card 1 of 90 cannot tell it apart from card 1 of 1 (AMUX-2533)")
    assert re.search(r"COUNT\(\*\)\s+n.*status='todo'", b, re.S), (
        "the depth is not computed from the same predicate pickup selects on")


def test_the_depth_query_shares_pickups_predicate():
    """A count that disagrees with the selector is worse than no count — it is the
    'view must share the predicate of the mechanism it describes' rule, and getting it
    wrong here would report a depth the lane will never actually be handed."""
    b = _pickup_block()
    for pred in ("status='todo'", "owner_type='agent'", "deleted IS NULL", "archived"):
        assert pred in b, "the depth query dropped %s, which pickup filters on" % pred


def test_a_real_backlog_says_so_rather_than_just_counting():
    """Below the threshold the count is context. Above it, grinding one-per-turn is the
    wrong SHAPE, and the notice has to say that — otherwise it reports a number and
    still leaves the lane to discover the problem 90 turns later."""
    b = _pickup_block()
    assert re.search(r"_qn\s*>=\s*10", b), "the backlog threshold is gone"
    assert "BACKLOG, not a work queue" in b
    assert "backlog` is never auto-picked" in b or "backlog is never auto-picked" in b, (
        "the notice does not name the honest re-shaping move (a real-but-not-ready card "
        "is `backlog`, which pickup never selects)")


def test_it_does_NOT_exempt_or_skip_anything():
    """The counter-case, and the constraint recorded on the card before any code was
    written: no cost- or lane-based exclusion. An exemption would make these cards
    silently undispatchable, which is how the `watch` type became invisible."""
    # COMMENTS STRIPPED. The first version searched the raw block and matched my own
    # comment explaining why I did NOT build an exemption ("a \"skip expensive lanes\"
    # rule would make cards silently undispatchable"). It failed against correct code,
    # which is the third instance of this shape tonight across two sessions: a probe
    # finding prose that DESCRIBES the defect and reading it as the defect. The repo
    # already strips comments for this reason in test_memory_archive_propagate_once.
    code = "\n".join(l for l in _pickup_block().splitlines()
                      if not l.lstrip().startswith("#"))
    for bad in ("skip", "exempt", "too expensive", "tokens_per_turn", "context_size"):
        assert bad not in code.lower(), (
            "pickup appears to exclude work based on %r — that is the exemption trap; "
            "the fix is information, not suppression" % bad)


def test_single_card_queues_stay_quiet():
    """No editorialising when there is nothing to editorialise about: the note is gated
    on _qn > 1, so an ordinary one-card pickup reads exactly as it did before."""
    b = _pickup_block()
    assert re.search(r"if\s+_qn\s*>\s*1\s*:", b), (
        "the queue note is no longer gated on there being a queue, so every ordinary "
        "single-card pickup now carries backlog advice it does not need")
