"""A destructive control must not wear the dismiss glyph (AMUX-2491).

"[amux/ui] Column delete uses the same ✕ affordance as modal-close, so a blind dismiss
loop can trigger a destructive confirm."

U+2715 is used 43 times in the client and almost all of them mean "close this, nothing
happens": search-clear, filter chips, chrome tab close, grid-pane close, modal close.
Three of them meant "delete this permanently". One of those three — the board column
delete — also rewrote `status` on every card in the column.

So a dismiss reflex, or an agent clicking every ✕ to clear overlays before doing its
real work, lands on a destructive confirm. A confirm dialog is a last line, not a
substitute for an affordance that never should have looked dismissible.

The codebase already had the right convention and had simply not applied it here: the
dictionary UI uses &#128465; (trash) with class="danger". This pins that convention so
the next delete button does not reach for ✕ again, which is the whole reason the
collision existed — nothing said not to.

Deliberately NOT asserted here: that every ✕ is non-destructive by some semantic
judgement. The test names the destructive controls explicitly. A test that tried to
infer destructiveness from the handler name would either miss things or fire on
`_crmDeleteIx`-shaped names that are fine, and a check that cannot be trusted gets
deleted rather than fixed.
"""

import re
from pathlib import Path

import pytest

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()

DISMISS_GLYPH = "&#x2715;"
TRASH_GLYPH = "&#128465;"

# Every control that PERMANENTLY destroys something, by class. Add to this list when
# you add a destructive control — that is the point of the list.
DESTRUCTIVE = [
    ("col-del-btn", "deletes a board column and rewrites the status of every card in it"),
    ("ws-profile-del", "deletes a saved browser profile"),
    ("crm-ix-del", "deletes a CRM interaction"),
]


def _button_html(cls):
    """The rendered markup for a control, by class. Returns every occurrence."""
    return re.findall(r"""<(?:button|span)[^>]*class=["']%s["'][^>]*>(?:[^<]*)""" % re.escape(cls), SRC)


@pytest.mark.parametrize("cls,what", DESTRUCTIVE)
def test_destructive_control_does_not_use_the_dismiss_glyph(cls, what):
    """THE REGRESSION. A control that %s must not render the same glyph as every
    close/dismiss button in the app."""
    found = _button_html(cls)
    assert found, "%s is gone from the client — remove it from DESTRUCTIVE or fix the class" % cls
    for html in found:
        assert DISMISS_GLYPH not in html, (
            "%s (%s) renders the dismiss glyph U+2715, the same one used by "
            "search-clear, filter chips, tab close and modal close. A dismiss loop "
            "reaches a destructive confirm: %s" % (cls, what, html[:160]))


@pytest.mark.parametrize("cls,what", DESTRUCTIVE)
def test_destructive_control_uses_the_trash_glyph(cls, what):
    """The positive half. Asserting only 'not ✕' would pass on a blank button or any
    other arbitrary glyph, so the convention itself is pinned — one shared symbol for
    delete, which is what makes it learnable."""
    for html in _button_html(cls):
        assert TRASH_GLYPH in html, (
            "%s (%s) does not use the trash glyph the rest of the app uses for "
            "delete: %s" % (cls, what, html[:160]))


def test_the_dismiss_glyph_is_still_widely_used_for_dismissal():
    """DENOMINATOR GUARD. The tests above are meaningful only because ✕ genuinely
    means 'dismiss' everywhere else — if someone purged U+2715 from the codebase
    entirely they would pass while the convention they encode had evaporated."""
    n = SRC.count(DISMISS_GLYPH)
    assert n >= 20, (
        "U+2715 has nearly vanished (%d uses); it no longer reads as 'the dismiss "
        "glyph', so the distinction these tests draw has stopped meaning anything" % n)


def test_column_delete_confirm_states_the_BLAST_RADIUS():
    """"Items will move to To Do" is true and withholds the only number that matters.
    An empty column and forty cards losing their status are the same sentence."""
    i = SRC.find("async function deleteBoardStatus")
    assert i > 0
    body = SRC[i:i + 1800]
    assert "boardItems" in body and "_statusCanon" in body, (
        "the confirm no longer counts the affected cards, so it cannot state how much "
        "the click destroys")
    assert "no undo" in body.lower(), (
        "the confirm no longer says the move is not undoable — a user agreeing to a "
        "bulk status rewrite should know it is one-way")


def test_column_delete_is_AUDITED_per_card():
    """What makes the affordance dangerous rather than untidy: the bulk rewrite used to
    leave NO trace — no History line, no interaction_log row, prior status overwritten.
    An accidental confirm was unrecoverable AND undiagnosable; you could not learn which
    cards had moved, let alone what they were (ethos rule 4).
    """
    # Anchor on the built-in guard, which is unique to the real handler. Anchoring
    # on the statuses regex found the ROUTE-LABEL MAPPER instead — the same pattern
    # appears there — and the test failed pointing at code that was never supposed
    # to have an audit call. The wrong-anchor failure is why this reads backwards
    # from a string only the handler contains.
    g = SRC.find("cannot delete built-in status")
    assert g > 0, "the column DELETE handler moved — re-anchor this test"
    i = SRC.rfind('if method == "DELETE":', 0, g)
    assert i > 0
    body = SRC[i:g + 2200]
    assert "_append_board_log" in body, "the per-card History line is gone"
    assert "_ilog(" in body, "the interaction_log audit row is gone"
    assert '"status"' in body and "before=" in body, (
        "the audit no longer records the PRIOR status, so the rewrite cannot be undone "
        "by hand — which is the only undo there is")
    assert "_hdr_worker" in body, (
        "the delete is no longer attributed; a bulk status rewrite must not be the one "
        "board mutation that lands unattributed")


def test_builtin_columns_still_cannot_be_deleted():
    """The pre-existing protection this change must not disturb: the seven built-in
    statuses are refused server-side regardless of what the UI offers."""
    i = SRC.find("cannot delete built-in status")
    assert i > 0, "the built-in guard is gone"
    guard = SRC[max(0, i - 400):i]
    for s in ("backlog", "todo", "doing", "review", "done", "verified", "discarded"):
        assert '"%s"' % s in guard, "%s is no longer protected from column delete" % s
