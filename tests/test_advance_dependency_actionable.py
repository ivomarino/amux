"""AC-298: don't nudge a lane to "drive its dependency through its gates" when it cannot.

Owning a card is not the same as being able to advance it. A card in `review` cannot be moved
by its AUTHOR — review->done needs the reviewer's sign-off and the gate rejects the author's
own ack. A card parked in `backlog` on an external trigger is waiting on the world.

The nudge checked only `dep.session == me`, so it fired EIGHT times in one session for AC-294:
fixed, committed, in review with reviewer amux-gtm, and the deploy that would verify it gated
on a human decision. The only way to silence it was to force a gate I could not satisfy — a
nudge meant to enforce the gates manufacturing pressure to lie to them.

HONEST LIMIT OF THIS FILE: these are source-shape assertions. The predicate is inline in
_advance_session, which needs live board rows and a running tmux session to drive end to end,
and I am not building that rig here. Two defects tonight (AC-300's dead `foreign` branch and
its false suppression) were in code that every source-shape test passed, so treat these as a
regression tripwire on the SHAPE, not as proof of behaviour. The behavioural check that was
actually run is recorded on the card: the server was reloaded with AC-294 genuinely in review,
and the suppression line appeared in the log on the next advance cycle.
"""
import re
from pathlib import Path

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _branch():
    """The same-session dependency branch, from the blocking check to the send_text."""
    i = SRC.index("is blocked by {_dep_id}")
    j = SRC.rindex("_blocking = _deps_blocking", 0, i)
    return SRC[j:i]


def test_the_gate_exists_and_precedes_the_send():
    b = _branch()
    assert "_why_stuck" in b, "the actionability gate is gone — AC-298 unchanged"
    assert b.rindex("return False") > b.index("_why_stuck"), (
        "the early return is before the gate computes, so nothing suppresses the nudge")


def test_review_is_treated_as_not_actionable_by_the_author():
    """THE INCIDENT. review->done requires a DIFFERENT worker, so the author has no honest
    move and the nudge must stay silent."""
    b = _branch()
    assert '_dstat == "review"' in b, (
        "a card in review no longer suppresses the nudge — its author will be told to drive "
        "a card only its reviewer can move")


def test_backlog_parked_on_a_trigger_is_not_actionable():
    b = _branch()
    assert '"backlog"' in b and "source_ref" in b, (
        "a card parked on an external trigger no longer suppresses — the lane will be nudged "
        "to work something that is waiting on the world")


def test_terminal_states_do_not_nudge_either():
    """A blocker that is already done/verified/discarded should not be 'driven' anywhere."""
    b = _branch()
    assert re.search(r'_dstat in \("done", "verified", "discarded"\)', b), (
        "a terminal blocker still produces a nudge")


def test_the_skip_is_logged():
    """A silent skip is indistinguishable from a nudge that was never due (ethos rule 4).
    When this eventually suppresses something it should NOT have, the log is the only way
    anyone finds out."""
    b = _branch()
    assert "not nudging (AC-298)" in b and "slog(" in b, (
        "the suppression leaves no trace, so an over-suppression is undiagnosable")
    assert "_why_stuck" in b[b.index("slog("):], (
        "the log line does not say WHICH condition suppressed it, so it cannot discriminate "
        "between the three reasons")
